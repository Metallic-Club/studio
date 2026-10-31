//! The developer overlay (Ctrl+Shift+F3, or `--dev-overlay`; #23): frame time and fps, GPU time,
//! the time per subsystem and a graph of the last frames, in the top-left corner over the game.
//!
//! Turning it on enables the profiler (`metallic_profiler`); turning it off disables it again,
//! so the overlay costs nothing while hidden. The numbers are formatted and laid out four times a
//! second, not every frame; only the graph is drawn fresh each frame. Colours are the Graphite
//! theme's tokens (`metallic_ui_theme`), since the overlay always sits over the game.
use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Id, LayerId, Order, Rect, Stroke, pos2, vec2,
};
use metallic_profiler::{FrameRecord, Profiler, Subsystem};
use metallic_ui_theme::{Palette, ThemeId, palette_of};

/// Frames averaged for the numbers (one second at 60 fps).
const AVERAGE: usize = 60;
/// Frames in the graph.
const GRAPH: usize = 180;
/// Frames between two layouts of the numbers (a quarter of a second at 60 fps).
const REFRESH: u32 = 15;
/// Height of one text row.
const ROW: f32 = 16.0;
/// Text rows: fps, GPU, then one per subsystem. The panel keeps this height so it never jumps.
const ROWS: usize = 2 + Subsystem::ALL.len();

/// The numbers the overlay shows, averaged over recent frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// Frames averaged.
    pub frames: usize,
    /// Mean frame time, milliseconds.
    pub frame_ms: f64,
    /// Slowest frame, milliseconds.
    pub max_ms: f64,
    /// Mean GPU time of the frames that have it, milliseconds.
    pub gpu_ms: Option<f64>,
    /// Mean exclusive time per subsystem, milliseconds, indexed like [`Subsystem::ALL`].
    pub subsystems_ms: [f64; Subsystem::ALL.len()],
}

impl Summary {
    /// Averages `frames` (any order).
    pub fn of<'a>(frames: impl Iterator<Item = &'a FrameRecord>) -> Option<Summary> {
        let mut n = 0usize;
        let mut total = 0u64;
        let mut max = 0u64;
        let mut gpu = (0u64, 0usize);
        let mut subsystems = [0u64; Subsystem::ALL.len()];
        for f in frames {
            n += 1;
            total += f.frame_ns();
            max = max.max(f.frame_ns());
            if let Some(g) = f.gpu_ns() {
                gpu = (gpu.0 + g, gpu.1 + 1);
            }
            for (sum, t) in subsystems.iter_mut().zip(f.subsystem_totals()) {
                *sum += t;
            }
        }
        if n == 0 {
            return None;
        }
        let ms = |ns: u64, count: usize| ns as f64 / 1e6 / count as f64;
        Some(Summary {
            frames: n,
            frame_ms: ms(total, n),
            max_ms: max as f64 / 1e6,
            gpu_ms: (gpu.1 > 0).then(|| ms(gpu.0, gpu.1)),
            subsystems_ms: subsystems.map(|t| ms(t, n)),
        })
    }

    /// Mean frames per second.
    pub fn fps(&self) -> f64 {
        if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        }
    }
}

/// One text row, laid out at the last refresh.
#[derive(Debug)]
struct Row {
    mark: Option<Color32>,
    name: Arc<Galley>,
    value: Arc<Galley>,
}

/// The overlay's on/off state.
#[derive(Debug, Default)]
pub struct DevOverlay {
    on: bool,
    /// Frame times of the graph, reused from frame to frame.
    graph: Vec<f32>,
    /// The text rows, laid out at the last refresh.
    rows: Vec<Row>,
    /// Frames painted since `rows` was laid out.
    age: u32,
}

impl DevOverlay {
    /// True while shown.
    #[cfg(test)]
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Shows or hides it (and starts or stops the profiler with it).
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        self.rows.clear();
        Profiler::set_enabled(on);
        if !on {
            Profiler::clear();
        }
    }

    /// Flips it (Ctrl+Shift+F3).
    pub fn toggle(&mut self) {
        self.set_on(!self.on);
    }

    /// Paints it into `ctx` (inside an egui pass), if shown.
    pub fn paint(&mut self, ctx: &egui::Context) {
        if !self.on {
            return;
        }
        // The same layer every frame: `Id::unique` hashes its source, it is not random (egui 0.37
        // renamed `Id::new` to it).
        let painter = ctx.layer_painter(LayerId::new(
            Order::Tooltip,
            Id::unique("metallic_dev_overlay"),
        ));
        let graph = &mut self.graph;
        graph.clear();
        let refresh = self.rows.is_empty() || self.age >= REFRESH;
        let summary = Profiler::with_frames(|frames| {
            graph.extend(
                frames
                    .iter()
                    .rev()
                    .take(GRAPH)
                    .map(|f| f.frame_ns() as f32 / 1e6),
            );
            refresh.then(|| Summary::of(frames.iter().rev().take(AVERAGE)))
        });
        graph.reverse();
        if let Some(summary) = summary {
            lay_out(&painter, summary.as_ref(), &mut self.rows);
            self.age = 0;
        }
        self.age += 1;
        draw(&painter, &self.rows, &self.graph);
    }
}

/// The overlay's colours.
fn palette() -> &'static Palette {
    palette_of(ThemeId::Graphite)
}

/// The mark of each subsystem: the same roles as Studio's profiler panel.
fn colour(p: &Palette, s: Subsystem) -> Color32 {
    match s {
        Subsystem::Script => p.accent,
        Subsystem::Physics => p.info,
        Subsystem::Character => p.success,
        Subsystem::Render => p.warning,
        Subsystem::Gui => p.text_2,
        Subsystem::Net => p.error,
        Subsystem::Other => p.text_3,
    }
}

/// Formats and lays out the text rows for `summary` into `rows`.
fn lay_out(painter: &egui::Painter, summary: Option<&Summary>, rows: &mut Vec<Row>) {
    let p = palette();
    let font = FontId::monospace(12.0);
    rows.clear();
    let mut add = |name: String, value: String, mark: Option<Color32>, ink: Color32| {
        rows.push(Row {
            mark,
            name: painter.layout_no_wrap(name, font.clone(), ink),
            value: painter.layout_no_wrap(value, font.clone(), ink),
        });
    };
    let Some(s) = summary else {
        add("Profiler".into(), "recording...".into(), None, p.text_1);
        return;
    };
    add(
        format!("{:.0} fps", s.fps()),
        format!("{:.2} ms (max {:.1})", s.frame_ms, s.max_ms),
        None,
        p.text_1,
    );
    add(
        "GPU".into(),
        s.gpu_ms
            .map_or("not measured".into(), |g| format!("{g:.2} ms")),
        None,
        p.text_1,
    );
    for (sub, ms) in Subsystem::ALL.iter().zip(s.subsystems_ms) {
        let ink = if ms > 0.0 { p.text_1 } else { p.text_3 };
        add(
            sub.label().into(),
            format!("{ms:.2} ms"),
            Some(colour(p, *sub)),
            ink,
        );
    }
}

fn draw(painter: &egui::Painter, rows: &[Row], graph: &[f32]) {
    let p = palette();
    let width = 260.0;
    let graph_h = 48.0;
    let height = 12.0 + ROWS as f32 * ROW + graph_h + 12.0;
    let rect = Rect::from_min_size(pos2(8.0, 8.0), vec2(width, height));
    painter.rect_filled(
        rect,
        CornerRadius::same(metallic_ui_theme::R_XS),
        p.bg_panel.gamma_multiply(0.9),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(metallic_ui_theme::R_XS),
        Stroke::new(1.0, p.line),
        egui::StrokeKind::Inside,
    );
    let top = rect.top() + 8.0;
    let left = rect.left() + 10.0;
    let right = rect.right() - 10.0;
    for (i, row) in rows.iter().enumerate() {
        let y = top + i as f32 * ROW;
        let mut x = left;
        if let Some(c) = row.mark {
            painter.rect_filled(
                Rect::from_min_size(pos2(x, y + 4.0), vec2(8.0, 8.0)),
                CornerRadius::same(2),
                c,
            );
            x += 14.0;
        }
        painter.galley(pos2(x, y), row.name.clone(), p.text_1);
        let value = Align2::RIGHT_TOP.anchor_size(pos2(right, y), row.value.size());
        painter.galley(value.min, row.value.clone(), p.text_1);
    }
    if graph.is_empty() {
        return;
    }
    let y = top + ROWS as f32 * ROW;
    let g = Rect::from_min_max(pos2(left, y + 4.0), pos2(right, y + 4.0 + graph_h));
    painter.rect_filled(g, CornerRadius::same(2), p.bg_deep);
    let peak = graph
        .iter()
        .copied()
        .fold(1000.0 / 30.0 * 1.1, f32::max)
        .min(100.0);
    for guide in [1000.0 / 60.0, 1000.0 / 30.0] {
        let gy = g.bottom() - guide / peak * g.height();
        painter.line_segment(
            [pos2(g.left(), gy), pos2(g.right(), gy)],
            Stroke::new(1.0, p.line),
        );
    }
    let bar = g.width() / GRAPH as f32;
    for (i, ms) in graph.iter().enumerate() {
        let x = g.right() - (graph.len() - i) as f32 * bar;
        let h = (ms / peak).min(1.0) * g.height();
        let c = if *ms > 1000.0 / 30.0 {
            p.error
        } else if *ms > 1000.0 / 58.0 {
            p.warning
        } else {
            p.accent
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(x, g.bottom() - h), pos2(x + bar.max(1.0), g.bottom())),
            0.0,
            c,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_averages_recent_frames() {
        Profiler::set_enabled(true);
        for _ in 0..3 {
            Profiler::begin_frame();
            {
                let _p = Profiler::scope(Subsystem::Physics, "physics");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Profiler::end_frame();
        }
        let s = Profiler::with_frames(|f| Summary::of(f.iter())).expect("frames");
        assert_eq!(s.frames, 3);
        assert!(s.frame_ms >= 2.0 && s.max_ms >= s.frame_ms);
        let physics = s.subsystems_ms[0..]
            .iter()
            .zip(Subsystem::ALL)
            .find(|(_, sub)| *sub == Subsystem::Physics);
        assert!(physics.is_some_and(|(ms, _)| *ms >= 2.0));
        assert_eq!(s.gpu_ms, None);
        assert!(s.fps() > 0.0 && s.fps() < 500.0);
        assert_eq!(Summary::of(std::iter::empty()), None);
    }

    #[test]
    fn toggling_starts_and_stops_the_profiler() {
        let mut overlay = DevOverlay::default();
        assert!(!Profiler::is_enabled());
        overlay.toggle();
        assert!(overlay.is_on() && Profiler::is_enabled());
        overlay.toggle();
        assert!(!overlay.is_on() && !Profiler::is_enabled());
        assert!(Profiler::with_frames(|f| f.is_empty()));
    }
}
