//! The performance overlay (Developer > Performance overlay, F3): fps and a frame-time graph,
//! heap memory, instance and Part counts, the viewport's draw calls and, while playing, the
//! script VMs' memory and waiting threads. It sits in the viewport's top-right corner and does not
//! take clicks.
//!
//! Per-subsystem timings are the Profiler's job (#23); the overlay only shows totals. While it
//! is shown Studio repaints every frame, so the numbers stay live.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use metallic_core::World;
use metallic_runtime::ScriptStats;

use crate::theme::{self, fonts};

/// Frame times kept for the graph.
pub const GRAPH_FRAMES: usize = 180;
/// Frames averaged for the fps figure.
pub const AVERAGE_FRAMES: usize = 60;
/// How often instances are counted again.
pub const COUNT_EVERY: Duration = Duration::from_millis(500);
/// Overlay's least width (it grows to fit its lines).
pub const WIDTH: f32 = 220.0;
const GRAPH_HEIGHT: f32 = 40.0;
/// The graph's top is this many milliseconds (two 30 fps frames).
const GRAPH_MS: f32 = 66.7;

static HEAP: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

/// The system allocator, counting the bytes in use so the overlay can show heap memory. The
/// Studio binary installs it with `#[global_allocator]`; without it the overlay shows no heap
/// figure.
pub struct CountingAlloc;

// SAFETY: every call goes straight to `System` with the same arguments; the counters are only
// bookkeeping and never affect what is returned.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is passed on unchanged.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            HEAP.fetch_add(layout.size(), Ordering::Relaxed);
            COUNTING.store(true, Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            HEAP.fetch_add(layout.size(), Ordering::Relaxed);
            COUNTING.store(true, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator (that is, from `System`) with `layout`.
        unsafe { System.dealloc(ptr, layout) };
        HEAP.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` came from `System` with `layout`; the caller upholds `realloc`'s rules.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            HEAP.fetch_add(new_size, Ordering::Relaxed);
            HEAP.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

/// Bytes on the heap now, or `None` when [`CountingAlloc`] is not the global allocator.
pub fn heap_bytes() -> Option<usize> {
    COUNTING
        .load(Ordering::Relaxed)
        .then(|| HEAP.load(Ordering::Relaxed))
}

/// The frame of the developer windows: raised panel, strong line, large radius, pop shadow.
pub fn window_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::bg_raised())
        .stroke(Stroke::new(1.0, theme::line_strong()))
        .corner_radius(theme::R_LG)
        .shadow(theme::SHADOW_POP)
        .inner_margin(12)
}

/// A developer window's title: the strong body font, not the large heading.
pub fn window_title(text: &str) -> RichText {
    RichText::new(text).font(fonts::strong())
}

/// Instances in a world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every instance under the roots.
    pub instances: usize,
    /// Those that are BaseParts.
    pub parts: usize,
}

/// Counts the instances and parts of `world`.
pub fn count(world: &World) -> Counts {
    let mut c = Counts::default();
    let mut stack: Vec<_> = world.roots().to_vec();
    while let Some(id) = stack.pop() {
        c.instances += 1;
        if world.is_a(id, "BasePart") {
            c.parts += 1;
        }
        stack.extend_from_slice(world.children(id));
    }
    c
}

/// Frame-time summary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSummary {
    /// Frames per second over the last [`AVERAGE_FRAMES`].
    pub fps: f32,
    /// Mean frame time in milliseconds.
    pub mean_ms: f32,
    /// Longest frame time in milliseconds.
    pub max_ms: f32,
}

/// What the overlay shows besides frame times, gathered by the editor each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayStats {
    /// Draw calls of the last viewport frame.
    pub draw_calls: Option<u32>,
    /// Script VMs while playing.
    pub scripts: Option<ScriptStats>,
    /// True while counts come from the play world.
    pub playing: bool,
}

/// The overlay's state: recent frame times and the last counts.
#[derive(Debug, Default)]
pub struct Overlay {
    frames: VecDeque<f32>,
    counts: Counts,
    counted_at: Option<Instant>,
}

impl Overlay {
    /// Records one frame that took `dt` seconds.
    pub fn record(&mut self, dt: f32) {
        if self.frames.len() >= GRAPH_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(dt.max(0.0));
    }

    /// Forgets the frame times (when the overlay is shown again).
    pub fn reset(&mut self) {
        self.frames.clear();
        self.counted_at = None;
    }

    /// The summary of the recorded frames.
    pub fn summary(&self) -> FrameSummary {
        let recent: Vec<f32> = self
            .frames
            .iter()
            .rev()
            .take(AVERAGE_FRAMES)
            .copied()
            .collect();
        if recent.is_empty() {
            return FrameSummary::default();
        }
        let mean = recent.iter().sum::<f32>() / recent.len() as f32;
        let max = recent.iter().copied().fold(0.0, f32::max);
        FrameSummary {
            fps: if mean > 0.0 { 1.0 / mean } else { 0.0 },
            mean_ms: mean * 1000.0,
            max_ms: max * 1000.0,
        }
    }

    /// True when the counts should be taken again (every [`COUNT_EVERY`]).
    pub fn counts_due(&self) -> bool {
        self.counted_at.is_none_or(|t| t.elapsed() >= COUNT_EVERY)
    }

    /// Stores fresh counts.
    pub fn set_counts(&mut self, counts: Counts) {
        self.counts = counts;
        self.counted_at = Some(Instant::now());
    }

    /// The last counts.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The overlay's text lines.
    pub fn lines(&self, stats: &OverlayStats) -> Vec<String> {
        let s = self.summary();
        let mut out = vec![format!(
            "{:.0} fps · {:.1} ms · max {:.0}",
            s.fps, s.mean_ms, s.max_ms
        )];
        out.push(match heap_bytes() {
            Some(b) => format!("Heap {}", mb(b)),
            None => "Heap not counted".to_owned(),
        });
        let world = if stats.playing { "Play" } else { "Edit" };
        out.push(format!(
            "{world}: {} parts · {} instances",
            self.counts.parts, self.counts.instances
        ));
        if let Some(d) = stats.draw_calls {
            out.push(format!("Draw calls {d}"));
        }
        if let Some(sc) = stats.scripts {
            for (name, vm) in [("Server", sc.server), ("Client", sc.client)] {
                if let Some(vm) = vm {
                    out.push(format!(
                        "{name} VM {} · {} waiting",
                        mb(vm.memory_bytes),
                        vm.waiting_threads
                    ));
                }
            }
            out.push(format!("{} scripts running", sc.running_scripts));
        }
        out
    }

    /// Draws the overlay in the top-right corner of `area` (the viewport).
    pub fn show(&self, ctx: &egui::Context, area: Rect, stats: &OverlayStats) {
        let pos = Pos2::new(area.right() - 12.0, area.top() + 12.0);
        egui::Area::new(egui::Id::unique("dev-overlay"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::bg_raised().gamma_multiply(0.92))
                    .stroke(Stroke::new(1.0, theme::line()))
                    .corner_radius(theme::R_MD)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_min_width(WIDTH - 16.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mono = egui::TextStyle::Monospace.resolve(ui.style());
                        for (i, line) in self.lines(stats).into_iter().enumerate() {
                            let color = if i == 0 {
                                theme::text_1()
                            } else {
                                theme::text_2()
                            };
                            ui.label(RichText::new(line).font(mono.clone()).color(color));
                            if i == 0 {
                                self.graph(ui);
                            }
                        }
                        ui.label(
                            RichText::new("F3 hides")
                                .font(fonts::small())
                                .color(theme::text_3()),
                        );
                    });
            });
    }

    fn graph(&self, ui: &mut egui::Ui) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(WIDTH - 16.0, GRAPH_HEIGHT), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 2.0, theme::bg_deep());
        let y_of = |ms: f32| rect.bottom() - (ms / GRAPH_MS).min(1.0) * rect.height();
        for (ms, color) in [(16.7, theme::success()), (33.3, theme::warning())] {
            let y = y_of(ms);
            p.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, color.gamma_multiply(0.5)),
            );
        }
        let w = rect.width() / GRAPH_FRAMES as f32;
        let start = GRAPH_FRAMES - self.frames.len();
        for (i, dt) in self.frames.iter().enumerate() {
            let ms = dt * 1000.0;
            let color: Color32 = if ms > 33.3 {
                theme::error()
            } else if ms > 16.7 {
                theme::warning()
            } else {
                theme::accent()
            };
            let x = rect.left() + (start + i) as f32 * w;
            p.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x, y_of(ms)),
                    Pos2::new(x + w.max(1.0), rect.bottom()),
                ),
                0.0,
                color,
            );
        }
    }
}

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
