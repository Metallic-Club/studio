//! The developer overlay (F3, or `--dev-overlay`; #23): frame time and fps, GPU time, the time
//! per subsystem and a graph of the last frames, in the top-left corner over the game.
//!
//! Turning it on enables the profiler (`metallic_profiler`); turning it off disables it again,
//! so the overlay costs nothing while hidden.
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Pos2, Rect, Stroke, pos2, vec2,
};
use metallic_profiler::{FrameRecord, Profiler, Subsystem};

/// Frames averaged for the numbers (one second at 60 fps).
const AVERAGE: usize = 60;
/// Frames in the graph.
const GRAPH: usize = 180;

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

/// The overlay's on/off state.
#[derive(Debug, Default)]
pub struct DevOverlay {
    on: bool,
    /// Frame times of the graph, reused from frame to frame.
    graph: Vec<f32>,
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
        Profiler::set_enabled(on);
        if !on {
            Profiler::clear();
        }
    }

    /// Flips it (F3).
    pub fn toggle(&mut self) {
        self.set_on(!self.on);
    }

    /// Paints it into `ctx` (inside an egui pass), if shown.
    pub fn paint(&mut self, ctx: &egui::Context) {
        if !self.on {
            return;
        }
        let graph = &mut self.graph;
        graph.clear();
        let summary = Profiler::with_frames(|frames| {
            graph.extend(
                frames
                    .iter()
                    .rev()
                    .take(GRAPH)
                    .map(|f| f.frame_ns() as f32 / 1e6),
            );
            Summary::of(frames.iter().rev().take(AVERAGE))
        });
        graph.reverse();
        let painter = ctx.layer_painter(LayerId::new(
            Order::Tooltip,
            Id::unique("metallic_dev_overlay"),
        ));
        draw(&painter, summary.as_ref(), graph);
    }
}

/// Restrained marks per subsystem (graphite UI, violet accent).
fn colour(s: Subsystem) -> Color32 {
    match s {
        Subsystem::Script => Color32::from_rgb(139, 108, 214),
        Subsystem::Physics => Color32::from_rgb(110, 150, 196),
        Subsystem::Character => Color32::from_rgb(120, 170, 140),
        Subsystem::Render => Color32::from_rgb(206, 170, 100),
        Subsystem::Gui => Color32::from_rgb(170, 176, 186),
        Subsystem::Net => Color32::from_rgb(196, 120, 120),
        Subsystem::Other => Color32::from_rgb(120, 124, 132),
    }
}

const PANEL: Color32 = Color32::from_rgba_premultiplied(18, 19, 22, 225);
const TEXT: Color32 = Color32::from_rgb(222, 224, 228);
const DIM: Color32 = Color32::from_rgb(150, 154, 162);
const LINE: Color32 = Color32::from_rgb(64, 66, 72);

fn draw(painter: &egui::Painter, summary: Option<&Summary>, graph: &[f32]) {
    let font = FontId::monospace(12.0);
    let width = 260.0;
    let rows = 2 + Subsystem::ALL.len();
    let graph_h = 48.0;
    let height = 12.0 + rows as f32 * 16.0 + graph_h + 12.0;
    let rect = Rect::from_min_size(pos2(8.0, 8.0), vec2(width, height));
    painter.rect_filled(rect, CornerRadius::same(4), PANEL);
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    let mut y = rect.top() + 8.0;
    let left = rect.left() + 10.0;
    let right = rect.right() - 10.0;
    let mut line = |text: String, value: String, colour: Option<Color32>, ink: Color32| {
        let mut x = left;
        if let Some(c) = colour {
            painter.rect_filled(
                Rect::from_min_size(pos2(x, y + 4.0), vec2(8.0, 8.0)),
                CornerRadius::same(2),
                c,
            );
            x += 14.0;
        }
        painter.text(Pos2::new(x, y), Align2::LEFT_TOP, text, font.clone(), ink);
        painter.text(
            Pos2::new(right, y),
            Align2::RIGHT_TOP,
            value,
            font.clone(),
            ink,
        );
        y += 16.0;
    };
    let Some(s) = summary else {
        line("Profiler".into(), "recording...".into(), None, TEXT);
        return;
    };
    line(
        format!("{:.0} fps", s.fps()),
        format!("{:.2} ms (max {:.1})", s.frame_ms, s.max_ms),
        None,
        TEXT,
    );
    line(
        "GPU".into(),
        s.gpu_ms
            .map_or("not measured".into(), |g| format!("{g:.2} ms")),
        None,
        TEXT,
    );
    for (sub, ms) in Subsystem::ALL.iter().zip(s.subsystems_ms) {
        let ink = if ms > 0.0 { TEXT } else { DIM };
        line(
            sub.label().into(),
            format!("{ms:.2} ms"),
            Some(colour(*sub)),
            ink,
        );
    }
    let g = Rect::from_min_max(pos2(left, y + 4.0), pos2(right, y + 4.0 + graph_h));
    painter.rect_filled(g, CornerRadius::same(2), Color32::from_rgb(10, 11, 13));
    let peak = graph
        .iter()
        .copied()
        .fold(1000.0 / 30.0 * 1.1, f32::max)
        .min(100.0);
    for guide in [1000.0 / 60.0, 1000.0 / 30.0] {
        let gy = g.bottom() - guide / peak * g.height();
        painter.line_segment(
            [pos2(g.left(), gy), pos2(g.right(), gy)],
            Stroke::new(1.0, LINE),
        );
    }
    let bar = g.width() / GRAPH as f32;
    for (i, ms) in graph.iter().enumerate() {
        let x = g.right() - (graph.len() - i) as f32 * bar;
        let h = (ms / peak).min(1.0) * g.height();
        let c = if *ms > 1000.0 / 30.0 {
            Color32::from_rgb(206, 120, 110)
        } else if *ms > 1000.0 / 58.0 {
            Color32::from_rgb(206, 170, 100)
        } else {
            Color32::from_rgb(139, 108, 214)
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
