//! The Profiler panel (#23): a graph of the last frames (stacked by subsystem, with 60 and 30 fps
//! lines), and for one frame its time per subsystem, its GPU stages, a timeline of its scopes and
//! its scripts sorted by time.
//!
//! Recording is off until the creator presses Record (`metallic_profiler` costs next to nothing
//! while off). Clicking a frame pauses the capture on it, so the history stops moving while it
//! is inspected; Resume records again. Script names come from the Play world while it runs and
//! are kept after Stop.
use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use metallic_core::{InstanceId, World};
use metallic_profiler::{FrameRecord, Profiler, ScopeRecord, Subsystem};

use crate::play::PlayView;
use crate::theme::{self, fonts};
use crate::widgets::{Button, segmented, truncated};

/// Frame times of the guide lines: 60 and 30 frames per second.
const GUIDES_MS: [(f32, &str); 2] = [(1000.0 / 60.0, "60 fps"), (1000.0 / 30.0, "30 fps")];
const GRAPH_HEIGHT: f32 = 72.0;
const MIN_BAR: f32 = 3.0;
const ROW: f32 = 18.0;
const TIMELINE_ROW: f32 = 18.0;

/// How the scripts list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOrder {
    /// Slowest first.
    Time,
    /// By full name.
    Name,
}

/// What the graph needs of one frame (copied each repaint; the full records stay in the
/// profiler).
#[derive(Debug, Clone, Copy)]
struct Bar {
    index: u64,
    frame_ns: u64,
    totals: [u64; Subsystem::ALL.len()],
}

/// The Profiler panel's state.
pub struct ProfilerPanel {
    recording: bool,
    /// The frame being inspected; the capture is paused while one is.
    selected: Option<u64>,
    order: ScriptOrder,
    /// Script ids (`InstanceId::to_bits`) to full names, filled from the Play world.
    names: HashMap<u64, String>,
    bars: Vec<Bar>,
    /// Pause on this frame when it is drawn (screenshots).
    pause_at: Option<u64>,
}

impl Default for ProfilerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfilerPanel {
    /// A panel that is not recording.
    pub fn new() -> ProfilerPanel {
        ProfilerPanel {
            recording: false,
            selected: None,
            order: ScriptOrder::Time,
            names: HashMap::new(),
            bars: Vec::new(),
            pause_at: None,
        }
    }

    /// Pauses on frame `index` once it has been recorded (design-review screenshots).
    pub fn pause_at(&mut self, index: u64) {
        self.pause_at = Some(index);
    }

    /// True while the profiler records (on and not paused on a frame).
    pub fn is_recording(&self) -> bool {
        self.recording && self.selected.is_none()
    }

    /// The frame being inspected, if the capture is paused on one.
    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    /// Starts or stops recording; stopping keeps the frames.
    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
        if on {
            self.selected = None;
        }
        self.apply();
    }

    /// Pauses the capture on frame `index` (or resumes with `None`).
    pub fn select(&mut self, index: Option<u64>) {
        self.selected = index;
        self.apply();
    }

    /// Sets how scripts are ordered.
    pub fn set_order(&mut self, order: ScriptOrder) {
        self.order = order;
    }

    fn apply(&self) {
        let on = self.is_recording();
        if on != Profiler::is_enabled() {
            Profiler::set_enabled(on);
        }
    }

    /// Draws the panel body. `play` is the running game, for script names.
    pub fn ui(&mut self, ui: &mut Ui, play: Option<&dyn PlayView>) {
        if let Some(at) = self.pause_at
            && Profiler::latest().is_some_and(|f| f.index() >= at + 3)
        {
            // A few frames past it, so its GPU time has arrived.
            self.pause_at = None;
            self.select(Some(at));
        }
        self.apply();
        self.bars.clear();
        Profiler::with_frames(|frames| {
            self.bars.extend(frames.iter().map(|f| Bar {
                index: f.index(),
                frame_ns: f.frame_ns(),
                totals: f.subsystem_totals(),
            }));
        });
        let shown = match self.selected {
            Some(i) => Profiler::with_frames(|frames| {
                frames.iter().rev().find(|f| f.index() == i).copied()
            }),
            None => Profiler::latest(),
        };
        if let (Some(frame), Some(view)) = (&shown, play) {
            self.learn_names(frame, view);
        }

        self.toolbar(ui, shown.as_ref());
        ui.add_space(4.0);
        self.graph(ui);
        ui.add_space(6.0);
        let Some(frame) = shown else {
            let hint = if self.recording {
                "Recording. Frames appear here as they are drawn."
            } else {
                "Press Record to measure frames: time per subsystem, GPU stages, scopes and scripts."
            };
            ui.label(
                egui::RichText::new(hint)
                    .font(fonts::small())
                    .color(theme::text_3()),
            );
            return;
        };
        let width = ui.available_width();
        let side = (width * 0.26).clamp(180.0, 280.0);
        let scripts = (width * 0.30).clamp(200.0, 360.0);
        let middle = (width - side - scripts - 24.0).max(120.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(vec2(side, ui.available_height()), |ui| {
                subsystems(ui, &frame);
                ui.add_space(6.0);
                gpu(ui, &frame, self.selected.is_none());
            });
            ui.add_space(12.0);
            ui.allocate_ui(vec2(middle, ui.available_height()), |ui| {
                timeline(ui, &frame);
            });
            ui.add_space(12.0);
            ui.allocate_ui(vec2(scripts, ui.available_height()), |ui| {
                self.scripts(ui, &frame);
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, shown: Option<&FrameRecord>) {
        ui.horizontal(|ui| {
            let label = if !self.recording {
                "Record"
            } else if self.selected.is_some() {
                "Resume"
            } else {
                "Pause"
            };
            if ui.add(Button::secondary(label).compact()).clicked() {
                if self.recording && self.selected.is_none() {
                    // Pause on the newest frame.
                    let newest = self.bars.last().map(|b| b.index);
                    match newest {
                        Some(i) => self.select(Some(i)),
                        None => self.set_recording(false),
                    }
                } else {
                    self.set_recording(true);
                }
            }
            if self.recording
                && ui
                    .add(Button::ghost("Stop").compact())
                    .on_hover_text("Stop recording and keep the frames")
                    .clicked()
            {
                self.set_recording(false);
            }
            if ui
                .add_enabled(!self.bars.is_empty(), Button::ghost("Clear").compact())
                .clicked()
            {
                Profiler::clear();
                self.bars.clear();
                self.selected = None;
                self.names.clear();
                self.apply();
            }
            ui.add_space(8.0);
            if let Some(f) = shown {
                let mut text = format!("Frame {} · {}", f.index(), ms(f.frame_ns()));
                if f.frame_ns() > 0 {
                    text.push_str(&format!(" · {:.0} fps", 1e9 / f.frame_ns() as f64));
                }
                if let Some(gpu) = f.gpu_ns() {
                    text.push_str(&format!(" · GPU {}", ms(gpu)));
                }
                ui.label(
                    egui::RichText::new(text)
                        .font(fonts::small())
                        .monospace()
                        .color(theme::text_2()),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for s in Subsystem::ALL.iter().rev() {
                    legend(ui, *s);
                }
            });
        });
    }

    /// The frame graph: one bar per frame, newest on the right; a click pauses on a frame.
    fn graph(&mut self, ui: &mut Ui) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_HEIGHT), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(theme::R_XS), theme::bg_deep());
        let fit = ((rect.width() / MIN_BAR) as usize).max(1);
        let start = self.bars.len().saturating_sub(fit);
        let bars = &self.bars[start..];
        let peak_ms = bars
            .iter()
            .map(|b| b.frame_ns as f32 / 1e6)
            .fold(GUIDES_MS[1].0 * 1.1, f32::max)
            .min(100.0);
        let y_of = |ms_value: f32| rect.bottom() - (ms_value / peak_ms).min(1.0) * rect.height();
        for (guide, label) in GUIDES_MS {
            let y = y_of(guide);
            painter.line_segment(
                [pos2(rect.left(), y), pos2(rect.right(), y)],
                Stroke::new(1.0, theme::line()),
            );
            painter.text(
                pos2(rect.left() + 4.0, y - 1.0),
                Align2::LEFT_BOTTOM,
                label,
                fonts::small(),
                theme::text_3(),
            );
        }
        let width = rect.width() / fit as f32;
        let hover_x = response.hover_pos().map(|p| p.x);
        let mut hovered = None;
        for (i, bar) in bars.iter().enumerate() {
            let left = rect.right() - (bars.len() - i) as f32 * width;
            let column =
                Rect::from_min_max(pos2(left, rect.top()), pos2(left + width, rect.bottom()));
            let mut bottom = rect.bottom();
            for (s, ns) in Subsystem::ALL.iter().zip(bar.totals) {
                if ns == 0 {
                    continue;
                }
                let h = ns as f32 / 1e6 / peak_ms * rect.height();
                let top = (bottom - h).max(rect.top());
                painter.rect_filled(
                    Rect::from_min_max(pos2(left, top), pos2(left + width - 1.0, bottom)),
                    0.0,
                    colour(*s),
                );
                bottom = top;
            }
            // Time outside any scope, up to the frame time.
            let top = y_of(bar.frame_ns as f32 / 1e6);
            if top < bottom {
                painter.rect_filled(
                    Rect::from_min_max(pos2(left, top), pos2(left + width - 1.0, bottom)),
                    0.0,
                    theme::line_strong(),
                );
            }
            if Some(bar.index) == self.selected {
                painter.rect_stroke(
                    column,
                    0.0,
                    Stroke::new(2.0, theme::accent()),
                    egui::StrokeKind::Inside,
                );
            }
            if hover_x.is_some_and(|x| column.x_range().contains(x)) {
                hovered = Some(*bar);
                painter.rect_filled(column, 0.0, theme::bg_hover().gamma_multiply(0.5));
            }
        }
        if let Some(bar) = hovered {
            response.clone().on_hover_text_at_pointer(format!(
                "Frame {} · {}",
                bar.index,
                ms(bar.frame_ns)
            ));
            if response.clicked() {
                self.select(Some(bar.index));
            }
        }
    }

    fn learn_names(&mut self, frame: &FrameRecord, view: &dyn PlayView) {
        if frame
            .scripts()
            .iter()
            .all(|s| self.names.contains_key(&s.script))
        {
            return;
        }
        let names = &mut self.names;
        view.inspect_world(&mut |world| {
            let mut stack: Vec<InstanceId> = world.roots().to_vec();
            while let Some(id) = stack.pop() {
                if world.class_name(id).is_some_and(|c| c.ends_with("Script")) {
                    names.insert(id.to_bits(), world.full_name(id));
                }
                stack.extend(world.children(id).iter().copied());
            }
        });
    }

    /// Learns script names from `world` (for hosts without a Play view, such as tests).
    pub fn learn_names_from(&mut self, world: &World) {
        let mut stack: Vec<InstanceId> = world.roots().to_vec();
        while let Some(id) = stack.pop() {
            if world.class_name(id).is_some_and(|c| c.ends_with("Script")) {
                self.names.insert(id.to_bits(), world.full_name(id));
            }
            stack.extend(world.children(id).iter().copied());
        }
    }

    fn scripts(&mut self, ui: &mut Ui, frame: &FrameRecord) {
        ui.horizontal(|ui| {
            section(ui, "Scripts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                segmented(
                    ui,
                    &mut self.order,
                    &[(ScriptOrder::Time, "Time"), (ScriptOrder::Name, "Name")],
                );
            });
        });
        let name_of = |id: u64| {
            self.names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("Script {id:#x}"))
        };
        let mut rows: Vec<(String, u64, u32)> = frame
            .scripts()
            .iter()
            .map(|s| (name_of(s.script), s.ns, s.calls))
            .collect();
        match self.order {
            ScriptOrder::Time => rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
            ScriptOrder::Name => rows.sort_by(|a, b| a.0.cmp(&b.0)),
        }
        if rows.is_empty() {
            dim(ui, "No script ran in this frame.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("profiler-scripts")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (name, ns, calls) in &rows {
                    let tip = format!("{name}\n{} in {calls} call(s)", ms(*ns));
                    value_row(ui, name, &ms(*ns), None).on_hover_text(tip);
                }
                if frame.other_scripts_ns() > 0 {
                    value_row(ui, "Other scripts", &ms(frame.other_scripts_ns()), None);
                }
            });
    }
}

/// The colour that marks `subsystem` in the graph, the legend and the timeline.
pub fn colour(subsystem: Subsystem) -> Color32 {
    match subsystem {
        Subsystem::Script => theme::accent(),
        Subsystem::Physics => theme::info(),
        Subsystem::Character => theme::success(),
        Subsystem::Render => theme::warning(),
        Subsystem::Gui => theme::highlight(),
        Subsystem::Net => theme::error(),
        Subsystem::Other => theme::text_3(),
    }
}

/// Milliseconds with two decimals (`1.23 ms`).
fn ms(ns: u64) -> String {
    format!("{:.2} ms", ns as f64 / 1e6)
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(fonts::caption())
            .strong()
            .color(theme::text_2()),
    );
}

fn dim(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(fonts::small())
            .color(theme::text_3()),
    );
}

fn legend(ui: &mut Ui, s: Subsystem) {
    ui.label(
        egui::RichText::new(s.label())
            .font(fonts::small())
            .color(theme::text_2()),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(2), colour(s));
    ui.add_space(6.0);
}

/// A row: a colour mark (optional), a name cut to fit, and a value on the right.
fn value_row(ui: &mut Ui, name: &str, value: &str, mark: Option<Color32>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::bg_hover());
    }
    let mut left = rect.left() + 2.0;
    if let Some(c) = mark {
        painter.rect_filled(
            Rect::from_center_size(pos2(left + 4.0, rect.center().y), vec2(8.0, 8.0)),
            CornerRadius::same(2),
            c,
        );
        left += 14.0;
    }
    let value_galley = painter.layout_no_wrap(value.to_owned(), fonts::small(), theme::text_1());
    let value_w = value_galley.size().x;
    painter.galley(
        pos2(
            rect.right() - value_w - 2.0,
            rect.center().y - value_galley.size().y / 2.0,
        ),
        value_galley,
        theme::text_1(),
    );
    let name_galley = truncated(
        painter,
        name,
        fonts::small(),
        theme::text_2(),
        rect.right() - value_w - 10.0 - left,
    );
    painter.galley(
        pos2(left, rect.center().y - name_galley.size().y / 2.0),
        name_galley,
        theme::text_2(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{name} {value}"))
    });
    response
}

fn subsystems(ui: &mut Ui, frame: &FrameRecord) {
    section(ui, "Subsystems");
    let totals = frame.subsystem_totals();
    for (s, ns) in Subsystem::ALL.iter().zip(totals) {
        if ns > 0 {
            value_row(ui, s.label(), &ms(ns), Some(colour(*s)));
        }
    }
    let unscoped = frame.unscoped_ns();
    if unscoped > 0 {
        value_row(ui, "Unscoped", &ms(unscoped), Some(theme::line_strong())).on_hover_text(
            "Frame time outside every scope: host work and waiting for the next frame.",
        );
    }
    if frame.dropped_scopes() > 0 {
        dim(
            ui,
            &format!("{} scopes not kept (frame limit)", frame.dropped_scopes()),
        );
    }
}

fn gpu(ui: &mut Ui, frame: &FrameRecord, live: bool) {
    section(ui, "GPU");
    match frame.gpu() {
        Some(passes) => {
            for p in passes {
                value_row(ui, p.name, &ms(p.ns), None);
            }
        }
        None if live => dim(ui, "Waiting for the GPU, or not measured on this device."),
        None => dim(
            ui,
            "Not measured (no timestamp queries, or no 3D view this frame).",
        ),
    }
}

/// The scopes of `frame` laid out in time, one row per nesting depth.
fn timeline(ui: &mut Ui, frame: &FrameRecord) {
    section(ui, "Timeline");
    let scopes = frame.scopes();
    if scopes.is_empty() {
        dim(ui, "No scopes in this frame.");
        return;
    }
    let depth = scopes.iter().map(|s| s.depth).max().unwrap_or(0) as f32 + 1.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), depth * TIMELINE_ROW),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::R_XS), theme::bg_deep());
    let span = frame.frame_ns().max(1) as f32;
    let x_of = |ns: u32| rect.left() + ns as f32 / span * rect.width();
    let mut hovered: Option<&ScopeRecord> = None;
    for s in scopes {
        let top = rect.top() + f32::from(s.depth) * TIMELINE_ROW;
        let r = Rect::from_min_max(
            pos2(x_of(s.start_ns), top + 1.0),
            pos2(
                x_of(s.start_ns.saturating_add(s.inclusive_ns)).max(x_of(s.start_ns) + 1.0),
                top + TIMELINE_ROW - 1.0,
            ),
        );
        painter.rect_filled(
            r,
            CornerRadius::same(2),
            colour(s.subsystem).gamma_multiply(0.75),
        );
        if r.width() > 24.0 {
            let g = truncated(
                &painter,
                s.name,
                fonts::small(),
                theme::text_1(),
                r.width() - 6.0,
            );
            painter.galley(
                pos2(r.left() + 3.0, r.center().y - g.size().y / 2.0),
                g,
                theme::text_1(),
            );
        }
        if response.hover_pos().is_some_and(|p| r.contains(p)) {
            hovered = Some(s);
        }
    }
    if let Some(s) = hovered {
        response.on_hover_text_at_pointer(format!(
            "{} ({})\n{} total, {} own",
            s.name,
            s.subsystem.label(),
            ms(u64::from(s.inclusive_ns)),
            ms(u64::from(s.exclusive_ns()))
        ));
    }
}
