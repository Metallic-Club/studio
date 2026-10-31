//! Move, rotate and scale gizmos: our own code, painted with egui shapes over the viewport image.
//!
//! # API for tools
//!
//! The gizmo is split into pure maths and painting so tools and tests can drive it without a
//! window:
//!
//! - [`GizmoFrame`]: origin, axes and on-screen size for one frame ([`GizmoFrame::new`]).
//! - [`GizmoFrame::hit`]: which [`Handle`] is under a screen point (constant ~90 px size).
//! - [`GizmoDrag`]: created on press with the start transforms of the parts it moves;
//!   [`GizmoDrag::update`] turns the current pointer ray into new [`PartTransform`]s (snapped
//!   relative to the start, Ctrl inverts snapping, Shift gives one-tenth precision);
//!   [`GizmoDrag::apply`] writes them to the world (live preview), [`GizmoDrag::cancel`] puts the
//!   start values back (Esc), and [`GizmoDrag::finish`] returns the single undo [`Command`] for the
//!   whole gesture ("Move 3 parts"), which the caller passes to `EditorState::record`.
//! - [`paint`] draws the handles; hovered and dragged handles thicken from 2.5 to 3.5 px; a drag
//!   shows its value next to the cursor ("X +4.00", "Y 35°").
//!
//! Scale moves one face (the opposite face stays put), with a 0.05 stud floor; Ball parts keep
//! equal axes. The building tools (#214) add axis lock, typed values and pivots on top of this.
use eframe::egui::{self, Color32, Pos2, Shape, Stroke};
use metallic_core::transform::MAX_PART_SIZE;
use metallic_core::{InstanceId, Value, World};
use metallic_render::glam::{DQuat, DVec3};

use crate::camera::ViewProjection;
use crate::commands::Command;
use crate::editor::{Snapping, Tool};
use crate::tokens::ui_tokens;
use crate::transform::{PartTransform, Ray, from_d, quat_to_orientation, snap};

/// Axis length on screen in points.
pub const AXIS_PX: f64 = 90.0;
/// Pointer distance (points) that still counts as on a handle.
const HIT_PX: f64 = 8.0;
/// Smallest size a resize handle gives, in studs (the move increment floor, spec 4.3). Core's
/// limits ([`metallic_core::transform::MIN_PART_SIZE`] .. [`MAX_PART_SIZE`]) still apply.
pub const MIN_SIZE: f64 = 0.05;
/// Ring sample count.
const RING_SEGMENTS: usize = 64;

/// A gizmo handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Move along axis 0, 1 or 2.
    Axis(usize),
    /// Move in the plane normal to an axis (the square takes that axis's colour).
    Plane(usize),
    /// Rotate about an axis.
    Ring(usize),
    /// Resize one face: axis and side.
    Face(usize, bool),
}

/// Where the gizmo is this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoFrame {
    /// World position of the gizmo (the pivot).
    pub origin: DVec3,
    /// Unit axes (world or the active part's local axes).
    pub axes: [DVec3; 3],
    /// Axis length in studs (so it looks [`AXIS_PX`] long).
    pub length: f64,
    /// Which handles exist.
    pub tool: Tool,
}

fn screen(view: &ViewProjection, p: DVec3) -> Option<(f64, f64)> {
    view.project(p)
}

fn dist_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-9 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

impl GizmoFrame {
    /// A gizmo at `origin` with `rotation` (identity for world space), sized for `view`.
    pub fn new(origin: DVec3, rotation: DQuat, tool: Tool, view: &ViewProjection) -> GizmoFrame {
        GizmoFrame {
            origin,
            axes: [
                rotation * DVec3::X,
                rotation * DVec3::Y,
                rotation * DVec3::Z,
            ],
            length: AXIS_PX * view.studs_per_point(origin),
            tool,
        }
    }

    /// Corners of the plane square for the plane normal to `axis`.
    pub fn plane_corners(&self, axis: usize) -> [DVec3; 4] {
        let (a, b) = (self.axes[(axis + 1) % 3], self.axes[(axis + 2) % 3]);
        let (s0, s1) = (self.length * 0.27, self.length * 0.40);
        let o = self.origin;
        [
            o + a * s0 + b * s0,
            o + a * s1 + b * s0,
            o + a * s1 + b * s1,
            o + a * s0 + b * s1,
        ]
    }

    /// True when the plane square normal to `axis` faces the camera enough to grab (an edge-on
    /// square would cover the arrows).
    pub fn plane_usable(&self, view: &ViewProjection, axis: usize) -> bool {
        let to_eye = (view.eye - self.origin).normalize_or(DVec3::Z);
        to_eye.dot(self.axes[axis]).abs() > 0.25
    }

    /// True when the arrow along `axis` does not point (almost) straight at the camera.
    pub fn arrow_usable(&self, view: &ViewProjection, axis: usize) -> bool {
        let to_eye = (view.eye - self.origin).normalize_or(DVec3::Z);
        to_eye.dot(self.axes[axis]).abs() < 0.98
    }

    /// Point `i` of `n` on the ring about `axis`.
    pub fn ring_point(&self, axis: usize, i: usize, n: usize) -> DVec3 {
        let (a, b) = (self.axes[(axis + 1) % 3], self.axes[(axis + 2) % 3]);
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        self.origin + (a * t.cos() + b * t.sin()) * self.length
    }

    /// Centre of a scale face handle.
    pub fn face_point(&self, axis: usize, positive: bool) -> DVec3 {
        let s = if positive { 1.0 } else { -1.0 };
        self.origin + self.axes[axis] * (self.length * s)
    }

    /// The handle under screen point `p`, nearest first.
    pub fn hit(&self, view: &ViewProjection, p: (f64, f64)) -> Option<Handle> {
        let mut best: Option<(f64, Handle)> = None;
        let mut consider = |d: f64, h: Handle| {
            if d <= HIT_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, h));
            }
        };
        let o = screen(view, self.origin)?;
        match self.tool {
            Tool::Select => {}
            Tool::Move => {
                for axis in 0..3 {
                    // Plane squares win over arrows when the pointer is inside them.
                    let c = self.plane_corners(axis);
                    if self.plane_usable(view, axis)
                        && let (Some(a), Some(b), Some(cc), Some(d)) = (
                            screen(view, c[0]),
                            screen(view, c[1]),
                            screen(view, c[2]),
                            screen(view, c[3]),
                        )
                        && inside_convex(&[a, b, cc, d], p)
                    {
                        consider(0.0, Handle::Plane(axis));
                    }
                    if self.arrow_usable(view, axis)
                        && let Some(tip) = screen(view, self.origin + self.axes[axis] * self.length)
                    {
                        let start = lerp2(o, tip, 0.18);
                        consider(dist_to_segment(p, start, tip), Handle::Axis(axis));
                    }
                }
            }
            Tool::Rotate => {
                for axis in 0..3 {
                    let mut prev = screen(view, self.ring_point(axis, 0, RING_SEGMENTS));
                    for i in 1..=RING_SEGMENTS {
                        let cur = screen(view, self.ring_point(axis, i, RING_SEGMENTS));
                        if let (Some(a), Some(b)) = (prev, cur) {
                            consider(dist_to_segment(p, a, b), Handle::Ring(axis));
                        }
                        prev = cur;
                    }
                }
            }
            Tool::Scale => {
                for axis in 0..3 {
                    for positive in [true, false] {
                        if let Some(c) = screen(view, self.face_point(axis, positive)) {
                            let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
                            consider((d - 2.0).max(0.0), Handle::Face(axis, positive));
                        }
                    }
                }
            }
        }
        best.map(|(_, h)| h)
    }
}

fn lerp2(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn inside_convex(poly: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut sign = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if cross.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Modifier state during a drag.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DragModifiers {
    /// Ctrl: invert snapping.
    pub invert_snap: bool,
    /// Shift: one-tenth speed.
    pub precision: bool,
}

/// One part moved by a drag, with its start values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragPart {
    /// The part.
    pub id: InstanceId,
    /// Transform when the drag started.
    pub start: PartTransform,
    /// `Orientation` value when the drag started (kept exactly for undo).
    pub start_orientation: metallic_core::Vec3,
    /// True for Ball parts (equal axes).
    pub ball: bool,
}

/// A gizmo drag in progress.
#[derive(Debug, Clone)]
pub struct GizmoDrag {
    /// The handle being dragged.
    pub handle: Handle,
    frame: GizmoFrame,
    parts: Vec<DragPart>,
    current: Vec<PartTransform>,
    /// Line parameter, plane point or ring vector where the drag started.
    start_param: f64,
    start_point: DVec3,
    /// Unwrapped ring angle (radians) and the last raw angle, for turns past 180 degrees.
    angle: f64,
    last_raw: f64,
    /// Value shown next to the cursor (studs, degrees or size delta).
    pub shown: f64,
}

impl GizmoDrag {
    /// Starts a drag of `handle` from the pointer `ray`; `None` when the ray does not meet the
    /// handle's line or plane (looking straight down an arrow).
    pub fn begin(
        world: &World,
        frame: GizmoFrame,
        handle: Handle,
        ray: &Ray,
        parts: &[InstanceId],
    ) -> Option<GizmoDrag> {
        let parts: Vec<DragPart> = parts
            .iter()
            .filter_map(|&id| {
                let start = PartTransform::read(world, id)?;
                let start_orientation = match world.get_ref(id, "Orientation") {
                    Ok(Value::Vector3(v)) => *v,
                    _ => return None,
                };
                let ball =
                    matches!(world.get_ref(id, "Shape"), Ok(Value::String(s)) if s == "Ball");
                Some(DragPart {
                    id,
                    start,
                    start_orientation,
                    ball,
                })
            })
            .collect();
        if parts.is_empty() {
            return None;
        }
        let mut drag = GizmoDrag {
            handle,
            frame,
            current: parts.iter().map(|p| p.start).collect(),
            parts,
            start_param: 0.0,
            start_point: DVec3::ZERO,
            angle: 0.0,
            last_raw: 0.0,
            shown: 0.0,
        };
        match handle {
            Handle::Axis(a) | Handle::Face(a, _) => {
                drag.start_param = ray.closest_on_line(frame.origin, frame.axes[a])?;
            }
            Handle::Plane(a) | Handle::Ring(a) => {
                let t = ray.plane_hit(frame.origin, frame.axes[a])?;
                drag.start_point = ray.at(t);
            }
        }
        Some(drag)
    }

    /// The transforms for the pointer at `ray`.
    pub fn update(
        &mut self,
        ray: &Ray,
        snapping: &Snapping,
        mods: DragModifiers,
    ) -> &[PartTransform] {
        let snap_on = snapping.enabled != mods.invert_snap;
        let speed = if mods.precision { 0.1 } else { 1.0 };
        let f = self.frame;
        match self.handle {
            Handle::Axis(a) => {
                let Some(s) = ray.closest_on_line(f.origin, f.axes[a]) else {
                    return &self.current;
                };
                let mut d = (s - self.start_param) * speed;
                if snap_on {
                    d = snap(d, snapping.move_step);
                }
                self.shown = d;
                let delta = f.axes[a] * d;
                for (cur, p) in self.current.iter_mut().zip(&self.parts) {
                    cur.position = p.start.position + delta;
                }
            }
            Handle::Plane(a) => {
                let Some(t) = ray.plane_hit(f.origin, f.axes[a]) else {
                    return &self.current;
                };
                let d = (ray.at(t) - self.start_point) * speed;
                let (u, v) = (f.axes[(a + 1) % 3], f.axes[(a + 2) % 3]);
                let (mut du, mut dv) = (d.dot(u), d.dot(v));
                if snap_on {
                    du = snap(du, snapping.move_step);
                    dv = snap(dv, snapping.move_step);
                }
                self.shown = (du * du + dv * dv).sqrt();
                let delta = u * du + v * dv;
                for (cur, p) in self.current.iter_mut().zip(&self.parts) {
                    cur.position = p.start.position + delta;
                }
            }
            Handle::Ring(a) => {
                let axis = f.axes[a];
                let Some(t) = ray.plane_hit(f.origin, axis) else {
                    return &self.current;
                };
                let v0 = self.start_point - f.origin;
                let v1 = ray.at(t) - f.origin;
                if v0.length_squared() < 1e-12 || v1.length_squared() < 1e-12 {
                    return &self.current;
                }
                let raw = axis.dot(v0.cross(v1)).atan2(v0.dot(v1));
                let mut step = raw - self.last_raw;
                if step > std::f64::consts::PI {
                    step -= std::f64::consts::TAU;
                } else if step < -std::f64::consts::PI {
                    step += std::f64::consts::TAU;
                }
                self.last_raw = raw;
                self.angle += step;
                let mut deg = self.angle.to_degrees() * speed;
                if snap_on {
                    deg = snap(deg, snapping.rotate_step);
                }
                self.shown = deg;
                let q = DQuat::from_axis_angle(axis, deg.to_radians());
                for (cur, p) in self.current.iter_mut().zip(&self.parts) {
                    cur.rotation = (q * p.start.rotation).normalize();
                    cur.position = f.origin + q * (p.start.position - f.origin);
                }
            }
            Handle::Face(a, positive) => {
                let Some(s) = ray.closest_on_line(f.origin, f.axes[a]) else {
                    return &self.current;
                };
                let sign = if positive { 1.0 } else { -1.0 };
                let mut d = (s - self.start_param) * sign * speed;
                if snap_on {
                    d = snap(d, snapping.move_step);
                }
                self.shown = d;
                for (cur, p) in self.current.iter_mut().zip(&self.parts) {
                    *cur = resize_face(p, f.axes[a], sign, d);
                }
            }
        }
        &self.current
    }

    /// Writes the current transforms into the world (live preview). Only the properties the
    /// handle changes are written: a move never rewrites `Orientation` or `Size`.
    pub fn apply(&self, world: &mut World) {
        let (rotates, resizes) = self.changes();
        for (cur, p) in self.current.iter().zip(&self.parts) {
            let _ = world.set(p.id, "Position", Value::Vector3(from_d(cur.position)));
            if rotates {
                let o = quat_to_orientation(cur.rotation);
                let _ = world.set(p.id, "Orientation", Value::Vector3(o));
            }
            if resizes {
                let _ = world.set(p.id, "Size", Value::Vector3(from_d(cur.size)));
            }
        }
    }

    /// Puts every part back where it started (Esc).
    pub fn cancel(&self, world: &mut World) {
        let (rotates, resizes) = self.changes();
        for p in &self.parts {
            let _ = world.set(p.id, "Position", Value::Vector3(from_d(p.start.position)));
            if rotates {
                let _ = world.set(p.id, "Orientation", Value::Vector3(p.start_orientation));
            }
            if resizes {
                let _ = world.set(p.id, "Size", Value::Vector3(from_d(p.start.size)));
            }
        }
    }

    /// (writes Orientation, writes Size) for this handle.
    fn changes(&self) -> (bool, bool) {
        match self.handle {
            Handle::Axis(_) | Handle::Plane(_) => (false, false),
            Handle::Ring(_) => (true, false),
            Handle::Face(..) => (false, true),
        }
    }

    /// The one undo step for the whole drag, or `None` when nothing changed.
    pub fn finish(&self, world: &World) -> Option<Command> {
        let mut commands = Vec::new();
        for p in &self.parts {
            for prop in ["Position", "Orientation", "Size"] {
                let old = match prop {
                    "Position" => Value::Vector3(from_d(p.start.position)),
                    "Orientation" => Value::Vector3(p.start_orientation),
                    _ => Value::Vector3(from_d(p.start.size)),
                };
                if let Ok(new) = world.get(p.id, prop)
                    && new != old
                {
                    commands.push(Command::set(p.id, prop, old, new));
                }
            }
        }
        if commands.is_empty() {
            return None;
        }
        let verb = match self.handle {
            Handle::Axis(_) | Handle::Plane(_) => "Move",
            Handle::Ring(_) => "Rotate",
            Handle::Face(..) => "Resize",
        };
        let n = self.parts.len();
        let noun = if n == 1 { "part" } else { "parts" };
        Some(Command::Batch {
            name: format!("{verb} {n} {noun}"),
            commands,
        })
    }

    /// Text next to the cursor.
    pub fn label(&self) -> String {
        let axis = |a: usize| ["X", "Y", "Z"][a];
        match self.handle {
            Handle::Axis(a) => format!("{} {:+.2}", axis(a), self.shown),
            Handle::Plane(_) => format!("{:.2} studs", self.shown),
            Handle::Ring(a) => format!("{} {:+.1}°", axis(a), self.shown),
            Handle::Face(a, _) => format!("Size {} {:+.2}", axis(a), self.shown),
        }
    }

    /// The frame the drag started with (painting keeps the gizmo where it was grabbed).
    pub fn frame(&self) -> GizmoFrame {
        self.frame
    }
}

/// Resizes one face of `p` along the gizmo axis `axis` (world space): the face on side `sign`
/// moves by `d`, the opposite face stays put. The part's local axis closest to `axis` is resized.
fn resize_face(p: &DragPart, axis: DVec3, sign: f64, d: f64) -> PartTransform {
    let mut t = p.start;
    let local = (0..3)
        .max_by(|&x, &y| {
            t.axis(x)
                .dot(axis)
                .abs()
                .total_cmp(&t.axis(y).dot(axis).abs())
        })
        .unwrap_or(0);
    if p.ball {
        let start = p.start.size.min_element();
        let new = (start + d).clamp(MIN_SIZE, MAX_PART_SIZE);
        t.size = DVec3::splat(new);
        t.position = p.start.position + axis * (sign * (new - start) * 0.5);
        return t;
    }
    let start = p.start.size[local];
    let new = (start + d).clamp(MIN_SIZE, MAX_PART_SIZE);
    t.size[local] = new;
    t.position = p.start.position + axis * (sign * (new - start) * 0.5);
    t
}

/// Axis colour.
pub fn axis_colour(axis: usize) -> Color32 {
    ui_tokens().axis[axis]
}

/// Draws the gizmo. `offset` is the viewport's top-left corner on screen.
pub fn paint(
    painter: &egui::Painter,
    offset: Pos2,
    view: &ViewProjection,
    frame: &GizmoFrame,
    hovered: Option<Handle>,
    drag: Option<&GizmoDrag>,
    pointer: Option<Pos2>,
) {
    let to = |p: DVec3| screen(view, p).map(|(x, y)| offset + egui::vec2(x as f32, y as f32));
    let active = drag.map(|d| d.handle).or(hovered);
    let width = |h: Handle| if active == Some(h) { 3.5 } else { 2.5 };
    let Some(o) = to(frame.origin) else { return };
    match frame.tool {
        Tool::Select => {}
        Tool::Move => {
            for axis in 0..3 {
                let c = axis_colour(axis);
                let plane = Handle::Plane(axis);
                let quad: Vec<Pos2> = frame
                    .plane_corners(axis)
                    .iter()
                    .filter_map(|&p| to(p))
                    .collect();
                if quad.len() == 4 && frame.plane_usable(view, axis) {
                    let alpha = if active == Some(plane) { 0.55 } else { 0.28 };
                    painter.add(Shape::convex_polygon(
                        quad,
                        c.gamma_multiply(alpha),
                        Stroke::new(1.0, c),
                    ));
                }
                if !frame.arrow_usable(view, axis) {
                    continue;
                }
                let Some(tip) = to(frame.origin + frame.axes[axis] * frame.length) else {
                    continue;
                };
                let start = o + (tip - o) * 0.18;
                painter.line_segment([start, tip], Stroke::new(width(Handle::Axis(axis)), c));
                arrow_head(painter, start, tip, c);
                painter.text(
                    tip + (tip - o).normalized() * 12.0,
                    egui::Align2::CENTER_CENTER,
                    ["X", "Y", "Z"][axis],
                    ui_tokens().small(),
                    c,
                );
            }
        }
        Tool::Rotate => {
            for axis in 0..3 {
                let c = axis_colour(axis);
                let pts: Vec<Pos2> = (0..=RING_SEGMENTS)
                    .filter_map(|i| to(frame.ring_point(axis, i, RING_SEGMENTS)))
                    .collect();
                painter.add(Shape::line(pts, Stroke::new(width(Handle::Ring(axis)), c)));
            }
            if let Some(d) = drag
                && let Handle::Ring(axis) = d.handle
            {
                swept_arc(painter, &to, frame, axis, d);
            }
        }
        Tool::Scale => {
            for axis in 0..3 {
                let c = axis_colour(axis);
                for positive in [true, false] {
                    let h = Handle::Face(axis, positive);
                    let Some(p) = to(frame.face_point(axis, positive)) else {
                        continue;
                    };
                    painter.line_segment([o, p], Stroke::new(1.5, c.gamma_multiply(0.7)));
                    let half = if active == Some(h) { 6.0 } else { 4.5 };
                    painter.rect_filled(
                        egui::Rect::from_center_size(p, egui::vec2(half, half) * 2.0),
                        1.0,
                        c,
                    );
                }
            }
        }
    }
    if let (Some(d), Some(p)) = (drag, pointer) {
        let text = d.label();
        let galley = painter.layout_no_wrap(text, ui_tokens().mono(), ui_tokens().text);
        let rect = egui::Rect::from_min_size(
            p + egui::vec2(16.0, 12.0),
            galley.size() + egui::vec2(10.0, 6.0),
        );
        painter.rect_filled(rect, 4.0, ui_tokens().panel);
        painter.galley(rect.min + egui::vec2(5.0, 3.0), galley, ui_tokens().text);
    }
}

fn arrow_head(painter: &egui::Painter, from: Pos2, tip: Pos2, c: Color32) {
    let dir = (tip - from).normalized();
    let side = egui::vec2(-dir.y, dir.x);
    let base = tip - dir * 11.0;
    painter.add(Shape::convex_polygon(
        vec![tip + dir * 2.0, base + side * 5.0, base - side * 5.0],
        c,
        Stroke::NONE,
    ));
}

fn swept_arc(
    painter: &egui::Painter,
    to: &dyn Fn(DVec3) -> Option<Pos2>,
    frame: &GizmoFrame,
    axis: usize,
    drag: &GizmoDrag,
) {
    let c = axis_colour(axis);
    let v0 = drag.start_point - frame.origin;
    let len = v0.length();
    if len < 1e-9 {
        return;
    }
    let v0 = v0 / len * frame.length;
    let steps = ((drag.shown.abs() / 5.0).ceil() as usize).clamp(1, 72);
    let mut mesh = egui::Mesh::default();
    let Some(centre) = to(frame.origin) else {
        return;
    };
    mesh.colored_vertex(centre, c.gamma_multiply(0.22));
    for i in 0..=steps {
        let a = (drag.shown * i as f64 / steps as f64).to_radians();
        let p = frame.origin + DQuat::from_axis_angle(frame.axes[axis], a) * v0;
        if let Some(sp) = to(p) {
            mesh.colored_vertex(sp, c.gamma_multiply(0.22));
        }
    }
    let n = mesh.vertices.len() as u32;
    for i in 1..n.saturating_sub(1) {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::EditorCamera;
    use metallic_core::Vec3;

    fn world_with_part(pos: Vec3, size: Vec3) -> (World, InstanceId) {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let p = w.create("Part", Some(ws)).unwrap();
        w.set(p, "Position", Value::Vector3(pos)).unwrap();
        w.set(p, "Size", Value::Vector3(size)).unwrap();
        (w, p)
    }

    fn frame(origin: DVec3, tool: Tool) -> GizmoFrame {
        GizmoFrame {
            origin,
            axes: [DVec3::X, DVec3::Y, DVec3::Z],
            length: 5.0,
            tool,
        }
    }

    fn ray_down(x: f64, z: f64) -> Ray {
        Ray {
            origin: DVec3::new(x, 50.0, z),
            dir: DVec3::NEG_Y,
        }
    }

    #[test]
    fn axis_drag_snaps_relative_to_start() {
        let (mut w, p) = world_with_part(Vec3::new(0.3, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::new(0.3, 0.0, 0.0), Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(0), &ray_down(0.3, 0.0), &[p]).unwrap();
        let snapping = Snapping::default();
        d.update(&ray_down(1.6, 0.0), &snapping, DragModifiers::default());
        d.apply(&mut w);
        let Value::Vector3(pos) = w.get(p, "Position").unwrap() else {
            panic!()
        };
        assert!(
            (pos.x - 1.3).abs() < 1e-9,
            "0.3 + 1 step = 1.3, got {}",
            pos.x
        );
        // Ctrl turns snapping off.
        d.update(
            &ray_down(1.6, 0.0),
            &snapping,
            DragModifiers {
                invert_snap: true,
                precision: false,
            },
        );
        assert!((d.current[0].position.x - 1.6).abs() < 1e-9);
        // Shift gives one tenth.
        d.update(
            &ray_down(10.3, 0.0),
            &snapping,
            DragModifiers {
                invert_snap: true,
                precision: true,
            },
        );
        assert!((d.current[0].position.x - 1.3).abs() < 1e-9);
    }

    #[test]
    fn move_keeps_an_unrounded_orientation_untouched() {
        let (mut w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        // A value quat_to_orientation would not give back bit for bit.
        let odd = Vec3::new(12.345678912, 33.3333333333, -7.1);
        w.set(p, "Orientation", Value::Vector3(odd)).unwrap();
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(0), &ray_down(0.0, 0.0), &[p]).unwrap();
        d.update(
            &ray_down(2.0, 0.0),
            &Snapping::default(),
            DragModifiers::default(),
        );
        d.apply(&mut w);
        let Some(Command::Batch { commands, .. }) = d.finish(&w) else {
            panic!()
        };
        assert_eq!(commands.len(), 1, "only Position changes: {commands:?}");
        assert_eq!(w.get(p, "Orientation").unwrap(), Value::Vector3(odd));
    }

    #[test]
    fn finish_is_one_step_and_cancel_restores() {
        let (mut w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(2), &ray_down(0.0, 0.0), &[p]).unwrap();
        d.update(
            &ray_down(0.0, 3.0),
            &Snapping::default(),
            DragModifiers::default(),
        );
        d.apply(&mut w);
        let cmd = d.finish(&w).unwrap();
        assert_eq!(cmd.name(&w), "Move 1 part");
        d.cancel(&mut w);
        assert_eq!(
            w.get(p, "Position").unwrap(),
            Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
        );
        assert!(d.finish(&w).is_none());
    }

    #[test]
    fn ring_drag_rotates_about_pivot_in_increments() {
        let (w, p) = world_with_part(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let f = frame(DVec3::ZERO, Tool::Rotate);
        // The Y ring lies in the plane y = 0; grab it at +X.
        let mut d = GizmoDrag::begin(&w, f, Handle::Ring(1), &ray_down(5.0, 0.0), &[p]).unwrap();
        // 40 degrees about +Y from +X goes toward -Z.
        let a = 40f64.to_radians();
        let cur = d.update(
            &ray_down(5.0 * a.cos(), -5.0 * a.sin()),
            &Snapping::default(),
            DragModifiers::default(),
        );
        // 40 snaps to 45 with a 15 degree increment.
        let expect = DQuat::from_rotation_y(45f64.to_radians()) * DVec3::new(2.0, 0.0, 0.0);
        assert!(
            (cur[0].position - expect).length() < 1e-9,
            "{:?}",
            cur[0].position
        );
        assert!((d.shown - 45.0).abs() < 1e-9);
    }

    #[test]
    fn ring_unwraps_past_half_a_turn() {
        let (w, p) = world_with_part(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let f = frame(DVec3::ZERO, Tool::Rotate);
        let mut d = GizmoDrag::begin(&w, f, Handle::Ring(1), &ray_down(5.0, 0.0), &[p]).unwrap();
        let s = Snapping {
            enabled: false,
            ..Snapping::default()
        };
        for deg in (10..=270).step_by(10) {
            let a = f64::from(deg).to_radians();
            d.update(
                &ray_down(5.0 * a.cos(), -5.0 * a.sin()),
                &s,
                DragModifiers::default(),
            );
        }
        assert!((d.shown - 270.0).abs() < 1e-6, "{}", d.shown);
    }

    #[test]
    fn face_drag_keeps_opposite_face_and_floors_size() {
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::ZERO, Tool::Scale);
        let mut d =
            GizmoDrag::begin(&w, f, Handle::Face(0, true), &ray_down(0.0, 0.0), &[p]).unwrap();
        let cur = d.update(
            &ray_down(2.0, 0.0),
            &Snapping::default(),
            DragModifiers::default(),
        )[0];
        assert!((cur.size.x - 6.0).abs() < 1e-9);
        // The -X face stays at -2.
        assert!((cur.position.x - cur.size.x / 2.0 - -2.0).abs() < 1e-9);
        let cur = d.update(
            &ray_down(-10.0, 0.0),
            &Snapping::default(),
            DragModifiers::default(),
        )[0];
        assert!((cur.size.x - MIN_SIZE).abs() < 1e-9);
        assert!((cur.position.x - cur.size.x / 2.0 - -2.0).abs() < 1e-9);
        // Core's largest part size caps the other end.
        let cur = d.update(
            &ray_down(5000.0, 0.0),
            &Snapping::default(),
            DragModifiers::default(),
        )[0];
        assert!((cur.size.x - MAX_PART_SIZE).abs() < 1e-9);
    }

    #[test]
    fn hit_finds_arrow_tips_on_screen() {
        let cam = EditorCamera::default();
        let view = cam.view((800.0, 600.0));
        let f = GizmoFrame::new(DVec3::ZERO, DQuat::IDENTITY, Tool::Move, &view);
        let tip = view.project(f.origin + DVec3::Y * f.length * 0.9).unwrap();
        assert_eq!(f.hit(&view, tip), Some(Handle::Axis(1)));
        assert_eq!(f.hit(&view, (5.0, 5.0)), None);
        let corners = f.plane_corners(1);
        let mid = view.project((corners[0] + corners[2]) * 0.5).unwrap();
        assert_eq!(f.hit(&view, mid), Some(Handle::Plane(1)));
    }
}
