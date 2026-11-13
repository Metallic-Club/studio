//! The code's minimap (#556), down the right of the script editor as VS Code has it: every line in small, two points
//! high and a point a character, in the code's colours, with the part in view shaded. It works as a scroll bar: a click
//! centres the code on the line clicked, and dragging the shaded part (or anywhere, after a click) scrolls.
//!
//! A script taller than the minimap scrolls in it too, in step with the code: at the top of the code the minimap shows
//! its first lines, at the bottom its last.

use egui::text::LayoutJob;
use egui::{Color32, Id, Painter, Rect, Response, Role, Sense, Ui, WidgetInfo, pos2};

/// The minimap's width.
pub const WIDTH: f32 = 96.0;

/// How tall a line is in it.
const LINE: f32 = 2.0;

/// How wide a character is in it.
const COLUMN: f32 = 1.0;

/// The room left of its first column.
const PAD: f32 = 6.0;

/// Where the minimap is and what it shows of the code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Map {
    /// Where it is on the screen.
    pub rect: Rect,
    /// How many lines the code has.
    pub lines: usize,
    /// How tall a line of the code is.
    pub row_height: f32,
    /// How tall the code's view is.
    pub view_height: f32,
    /// How far down the code is scrolled.
    pub offset: f32,
    /// How far down it can be scrolled.
    pub max_offset: f32,
}

impl Map {
    /// How tall the shaded part is: the lines in view.
    pub fn slider_height(&self) -> f32 {
        (self.view_height / self.row_height * LINE).min(self.rect.height())
    }

    /// How far the code is scrolled, from 0 (top) to 1 (bottom).
    fn fraction(&self) -> f32 {
        if self.max_offset <= 0.0 {
            0.0
        } else {
            (self.offset / self.max_offset).clamp(0.0, 1.0)
        }
    }

    /// How far the shaded part can move: down the lines drawn, or the minimap when they fill it.
    fn track(&self) -> f32 {
        ((self.lines as f32 * LINE).min(self.rect.height()) - self.slider_height()).max(0.0)
    }

    /// How far the minimap itself is scrolled, when the code is taller than it.
    fn scroll(&self) -> f32 {
        self.fraction() * (self.lines as f32 * LINE - self.rect.height()).max(0.0)
    }

    /// The shaded part, over the lines in view.
    pub fn slider(&self) -> Rect {
        let top = self.rect.top() + self.fraction() * self.track();
        Rect::from_min_max(
            pos2(self.rect.left(), top),
            pos2(self.rect.right(), top + self.slider_height()),
        )
    }

    /// The top of line `line` (from 0) in it.
    pub fn line_top(&self, line: usize) -> f32 {
        self.rect.top() + line as f32 * LINE - self.scroll()
    }

    /// The code's offset that puts the shaded part's top at `top`.
    pub fn offset_for_slider(&self, top: f32) -> f32 {
        let track = self.track();
        if track <= 0.0 {
            return 0.0;
        }
        ((top - self.rect.top()) / track).clamp(0.0, 1.0) * self.max_offset.max(0.0)
    }

    /// The code's offset that centres the line at `y` in the view.
    pub fn offset_centring(&self, y: f32) -> f32 {
        let line = (y - self.rect.top() + self.scroll()) / LINE;
        (line * self.row_height - self.view_height / 2.0).clamp(0.0, self.max_offset.max(0.0))
    }
}

/// Paints the code laid out by `job` (a tab `tab_width` characters wide) in `map`, each run of characters in its colour,
/// and the shaded part over the lines in view, brighter while `active`.
pub fn paint(
    painter: &Painter,
    map: &Map,
    job: &LayoutJob,
    tab_width: usize,
    colors: (Color32, Color32),
    active: bool,
) {
    let (ground, ink) = colors;
    let painter = painter.with_clip_rect(map.rect.intersect(painter.clip_rect()));
    painter.rect_filled(map.rect, 0.0, ground);
    let first = (map.scroll() / LINE).floor().max(0.0) as usize;
    let last = first + (map.rect.height() / LINE).ceil() as usize + 1;
    let right = map.rect.right() - 2.0;
    let tab = tab_width.max(1);
    let (mut line, mut column) = (0usize, 0usize);
    // A run of characters in one colour on the line: its first and past-last column.
    let mut run: Option<(usize, usize, Color32)> = None;
    let flush = |run: &mut Option<(usize, usize, Color32)>, line: usize| {
        if let Some((from, to, color)) = run.take() {
            let left = map.rect.left() + PAD + from as f32 * COLUMN;
            if left < right {
                let top = map.line_top(line);
                let rect = Rect::from_min_max(
                    pos2(left, top),
                    pos2(
                        (map.rect.left() + PAD + to as f32 * COLUMN).min(right),
                        top + LINE * 0.8,
                    ),
                );
                painter.rect_filled(rect, 0.0, color.gamma_multiply(0.7));
            }
        }
    };
    'sections: for section in &job.sections {
        let color = section.format.color;
        let Some(text) = job.text.get(section.byte_range.start.0..section.byte_range.end.0) else {
            continue;
        };
        for c in text.chars() {
            match c {
                '\n' => {
                    flush(&mut run, line);
                    line += 1;
                    column = 0;
                    if line > last {
                        break 'sections;
                    }
                }
                '\t' => {
                    flush(&mut run, line);
                    column = (column / tab + 1) * tab;
                }
                c if c.is_whitespace() => {
                    flush(&mut run, line);
                    column += 1;
                }
                _ => {
                    if line >= first {
                        match &mut run {
                            Some((_, to, same)) if *same == color && *to == column => *to += 1,
                            _ => {
                                flush(&mut run, line);
                                run = Some((column, column + 1, color));
                            }
                        }
                    }
                    column += 1;
                }
            }
        }
    }
    flush(&mut run, line);
    let alpha = if active { 0.16 } else { 0.08 };
    painter.rect_filled(map.slider(), 0.0, ink.gamma_multiply(alpha));
}

/// The minimap `map` as a scroll bar, interacting as `id`: returns the code's offset a click or a drag asks for.
pub fn interact(ui: &Ui, id: Id, map: &Map) -> (Response, Option<f32>) {
    let response = ui.interact(map.rect, id, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(Role::ScrollBar, true, "Minimap"));
    let grab_id = id.with("grab");
    let pointer = response.interact_pointer_pos();
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let mut asked = None;
    if pressed && let Some(at) = ui.input(|input| input.pointer.interact_pos()) {
        let slider = map.slider();
        // On the shaded part, it is held where it was taken; elsewhere, the code centres on the line clicked.
        let grab = if slider.contains(at) {
            at.y - slider.top()
        } else {
            asked = Some(map.offset_centring(at.y));
            map.slider_height() / 2.0
        };
        ui.ctx().data_mut(|data| data.insert_temp(grab_id, grab));
    } else if response.dragged()
        && let Some(at) = pointer
    {
        let grab = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(grab_id))
            .unwrap_or(map.slider_height() / 2.0);
        asked = Some(map.offset_for_slider(at.y - grab));
    }
    (response, asked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn map(lines: usize, offset: f32) -> Map {
        let row_height = 20.0;
        let view_height = 400.0;
        Map {
            rect: Rect::from_min_size(pos2(500.0, 100.0), vec2(WIDTH, view_height)),
            lines,
            row_height,
            view_height,
            offset,
            max_offset: (lines as f32 * row_height - view_height).max(0.0),
        }
    }

    #[test]
    fn a_short_script_s_shaded_part_covers_the_lines_in_view() {
        // 100 lines fit in the minimap (200 points of 400); 20 are in view.
        let top = map(100, 0.0);
        assert_eq!(top.slider_height(), 40.0);
        assert_eq!(top.slider().top(), 100.0);
        // Scrolled 30 lines down: the shaded part is over line 30.
        let down = map(100, 600.0);
        assert_eq!(down.slider().top(), down.line_top(30));
        assert_eq!(down.line_top(0), 100.0, "the minimap does not scroll");
    }

    #[test]
    fn a_long_script_s_minimap_scrolls_with_it() {
        // 1000 lines: 2000 points of minimap in 400.
        let bottom = map(1000, 1000.0 * 20.0 - 400.0);
        assert_eq!(bottom.slider().bottom(), bottom.rect.bottom());
        // The last line is at the minimap's bottom.
        assert!((bottom.line_top(1000) - bottom.rect.bottom()).abs() < 0.01);
        // The shaded part covers the last lines in view.
        assert!((bottom.slider().top() - bottom.line_top(980)).abs() < 0.01);
        let middle = map(1000, 9800.0);
        assert!((middle.slider().top() - middle.line_top(490)).abs() < 0.01);
    }

    #[test]
    fn dragging_and_clicking_ask_for_an_offset() {
        let m = map(1000, 0.0);
        assert_eq!(m.offset_for_slider(m.rect.top()), 0.0);
        assert_eq!(m.offset_for_slider(m.rect.bottom()), m.max_offset);
        let half = m.offset_for_slider(m.rect.top() + (m.rect.height() - m.slider_height()) / 2.0);
        assert!((half - m.max_offset / 2.0).abs() < 0.01);
        // A click on line 100 of a short script centres it.
        let short = map(150, 0.0);
        let y = short.line_top(100) + 1.0;
        assert!((short.offset_centring(y) - (100.5 * 20.0 - 200.0)).abs() < 0.01);
        // Not past the ends.
        assert_eq!(short.offset_centring(short.rect.top()), 0.0);
    }
}
