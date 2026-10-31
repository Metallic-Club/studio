//! Move, rotate, scale and Transform gizmos, body drags and pivot editing: our own code, painted
//! with egui shapes over the viewport image.
//!
//! # API for tools
//!
//! The gizmo is split into pure maths and painting so tools and tests can drive it without a
//! window:
//!
//! - [`GizmoFrame`]: origin (the pivot), axes and on-screen size for one frame.
//! - [`GizmoFrame::hit`]: which [`Handle`] is under a screen point (constant ~90 px size).
//! - [`GizmoDrag`]: one drag. [`GizmoDrag::begin`] for a handle, [`GizmoDrag::begin_body`] for
//!   dragging the parts themselves (they follow the surface under the cursor, section 5.2), and
//!   [`GizmoDrag::begin_pivot`] to move only the pivot (P, section 4.4).
//!   [`GizmoDrag::update`] turns the pointer into new transforms; [`GizmoDrag::apply`] writes them
//!   through the [`PreviewOverlay`]; [`GizmoDrag::finish`] returns the single undo step for the
//!   whole gesture ("Move 3 parts", "Duplicate and move"); [`GizmoDrag::cancel`] restores
//!   everything (Esc).
//! - During a drag: [`GizmoDrag::lock`] (X/Y/Z lock to a world axis, Shift for the plane),
//!   [`GizmoDrag::turn`] (R/T turn the body 90 degrees), typed values ([`GizmoDrag::type_char`],
//!   [`GizmoDrag::commit_typed`]: "5 Enter" moves 5 studs on the dragged axis, `3*4` moves 12),
//!   and [`GizmoDrag::set_amount`] to change the amount after release (the editable delta).
//! - [`paint`] draws the handles; hovered and dragged handles thicken from 2.5 to 3.5 px; a drag
//!   shows its value next to the cursor ("X +4.00", "Y 35°", "x1.50").
//!
//! Handles: arrows, plane squares and a centre disc (Move); rings (Rotate); face cubes and a
//! corner square for uniform scale (Scale); all of them at once (Transform). Snapping is relative
//! to the start, Ctrl inverts it, Shift gives one tenth. A face drag moves one face (Alt: both),
//! within core's 0.001 to 2048 stud limits; Ball parts keep equal axes. Point snapping (vertices, edges; V held)
//! comes from [`crate::snapping`].
use std::collections::HashSet;

use eframe::egui::{self, Color32, Pos2, Shape, Stroke};
use metallic_core::transform::{MAX_PART_SIZE, MIN_PART_SIZE};
use metallic_core::{InstanceId, Value, World};
use metallic_render::glam::{DQuat, DVec3};

use crate::camera::ViewProjection;
use crate::commands::{Command, PropName, count};
use crate::editor::{Snapping, Tool};
use crate::picking::PickCache;
use crate::pivot::{Pivot, Rigid, pivot_writes, to_cframe};
use crate::preview::PreviewOverlay;
use crate::snapping::{
    self, SNAP_RADIUS_PX, SnapCandidate, SnapKind, SnapKinds, SnapQuery, SnapSource, Surface,
    place_on_surface,
};
use crate::tokens::ui_tokens;
use crate::transform::{PartTransform, Ray, from_d, quat_to_orientation, snap};

/// Axis length on screen in points.
pub const AXIS_PX: f64 = 90.0;
/// Pointer distance (points) that still counts as on a handle.
const HIT_PX: f64 = 8.0;
/// Smallest size a resize handle gives, in studs: core's limit, so a handle never changes a size
/// it did not drag (spec 4.3).
pub const MIN_SIZE: f64 = MIN_PART_SIZE;
/// Ring sample count.
const RING_SEGMENTS: usize = 64;
/// Radius of the centre disc in points (8 px across).
const DISC_PX: f64 = 4.0;
/// Half size of the uniform-scale corner square in points (9 px across).
const CORNER_PX: f64 = 4.5;

/// A gizmo handle (or the body of the selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Move along axis 0, 1 or 2.
    Axis(usize),
    /// Move in the plane normal to an axis (the square takes that axis's colour).
    Plane(usize),
    /// Move freely in the screen plane (centre disc).
    Free,
    /// Rotate about an axis.
    Ring(usize),
    /// Resize one face: axis and side.
    Face(usize, bool),
    /// Scale the whole selection about the pivot (corner square).
    Uniform,
    /// The parts themselves: they follow the surface under the cursor.
    Body,
}

/// X/Y/Z lock during a drag (world axes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Move along this world axis only.
    Axis(usize),
    /// Move in the plane normal to this world axis.
    Plane(usize),
}

/// Where the gizmo is this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoFrame {
    /// World position of the gizmo (the pivot).
    pub origin: DVec3,
    /// Unit axes (world or the pivot's local axes).
    pub axes: [DVec3; 3],
    /// Axis length in studs (so it looks [`AXIS_PX`] long).
    pub length: f64,
    /// Which handles exist.
    pub tool: Tool,
}

fn screen(view: &ViewProjection, p: DVec3) -> Option<(f64, f64)> {
    view.project(p)
}

fn dist2(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn dist_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-9 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    dist2(p, (a.0 + dx * t, a.1 + dy * t))
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

    fn has_move(&self) -> bool {
        matches!(self.tool, Tool::Move | Tool::Transform)
    }

    fn has_rotate(&self) -> bool {
        matches!(self.tool, Tool::Rotate | Tool::Transform)
    }

    fn has_scale(&self) -> bool {
        matches!(self.tool, Tool::Scale | Tool::Transform)
    }

    /// Ring radius in studs (outside the arrows for Transform).
    pub fn ring_radius(&self) -> f64 {
        if self.tool == Tool::Transform {
            self.length * 1.25
        } else {
            self.length
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
        self.origin + (a * t.cos() + b * t.sin()) * self.ring_radius()
    }

    /// Centre of a scale face handle.
    pub fn face_point(&self, axis: usize, positive: bool) -> DVec3 {
        let s = if positive { 1.0 } else { -1.0 };
        let d = if self.tool == Tool::Transform {
            self.length * 0.62
        } else {
            self.length
        };
        self.origin + self.axes[axis] * (d * s)
    }

    /// Centre of the uniform-scale corner square (+X+Y+Z in the gizmo's axes).
    pub fn corner_point(&self) -> DVec3 {
        self.origin + (self.axes[0] + self.axes[1] + self.axes[2]) * (self.length * 0.5)
    }

    /// The handle under screen point `p`, nearest first. Allocates nothing.
    pub fn hit(&self, view: &ViewProjection, p: (f64, f64)) -> Option<Handle> {
        let mut best: Option<(f64, Handle)> = None;
        let mut consider = |d: f64, h: Handle| {
            if d <= HIT_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, h));
            }
        };
        let o = screen(view, self.origin)?;
        if self.has_move() {
            consider((dist2(p, o) - DISC_PX).max(0.0), Handle::Free);
        }
        if self.has_scale() {
            if let Some(c) = screen(view, self.corner_point()) {
                consider((dist2(p, c) - CORNER_PX).max(0.0), Handle::Uniform);
            }
            for axis in 0..3 {
                for positive in [true, false] {
                    if let Some(c) = screen(view, self.face_point(axis, positive)) {
                        consider((dist2(p, c) - 2.0).max(0.0), Handle::Face(axis, positive));
                    }
                }
            }
        }
        if self.has_move() {
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
        if self.has_rotate() {
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
    /// Alt: a face drag moves both opposite faces.
    pub alt: bool,
}

/// What one frame of a drag sees.
#[derive(Clone, Copy)]
pub struct DragInput<'a> {
    /// Pointer ray.
    pub ray: Ray,
    /// Pointer in viewport points.
    pub cursor: (f64, f64),
    /// The camera (needed for point snapping).
    pub view: Option<&'a ViewProjection>,
    /// The scene's parts (surface and point snapping).
    pub picks: Option<&'a PickCache>,
    /// Header snapping settings.
    pub snapping: &'a Snapping,
    /// Ctrl, Shift and Alt.
    pub mods: DragModifiers,
    /// V held: vertex and edge snapping.
    pub vertex: bool,
    /// V+Shift held: the grab vertex goes onto the surface.
    pub onto_surface: bool,
}

impl<'a> DragInput<'a> {
    /// Input without a scene (tests and handle-only drags).
    pub fn simple(ray: Ray, snapping: &'a Snapping, mods: DragModifiers) -> DragInput<'a> {
        DragInput {
            ray,
            cursor: (0.0, 0.0),
            view: None,
            picks: None,
            snapping,
            mods,
            vertex: false,
            onto_surface: false,
        }
    }
}

/// One part moved by a drag, with its start values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragPart {
    /// The part.
    pub id: InstanceId,
    /// Transform when the drag started.
    pub start: PartTransform,
    /// `Orientation` value when the drag started (written back exactly when unrotated).
    pub start_orientation: metallic_core::Vec3,
    /// True for Ball parts (equal axes).
    pub ball: bool,
}

#[derive(Debug, Clone)]
enum Target {
    Parts,
    Pivot {
        id: InstanceId,
        name: String,
        start: Pivot,
        points: Box<[DVec3; 27]>,
    },
}

#[derive(Debug, Clone)]
struct Body {
    /// Selection box centre at the start.
    r0: DVec3,
    /// Every part's corners relative to `r0`, unrotated.
    corners: Vec<DVec3>,
    /// The same, rotated this frame (scratch, reused).
    rotated: Vec<DVec3>,
    /// Grab point relative to `r0`.
    grab: DVec3,
    /// The active part's up axis at the start.
    up: DVec3,
    /// R / T turns so far.
    user: DQuat,
    /// The last real surface touched (for the virtual surface).
    last: Option<Surface>,
    /// Start height of the selection's bottom (virtual ground plane).
    ground: f64,
}

#[derive(Debug, Clone, Copy)]
enum Constraint {
    Line(DVec3),
    Plane(DVec3, DVec3, DVec3),
    Free(DVec3),
}

const POSITION: usize = 0;
const ORIENTATION: usize = 1;
const SIZE: usize = 2;

/// A drag in progress (handle, body or pivot).
#[derive(Debug, Clone)]
pub struct GizmoDrag {
    /// The handle being dragged.
    pub handle: Handle,
    frame: GizmoFrame,
    about: DVec3,
    target: Target,
    parts: Vec<DragPart>,
    current: Vec<PartTransform>,
    ids: HashSet<InstanceId>,
    start_ray: Ray,
    free_normal: DVec3,
    start_param: f64,
    start_point: DVec3,
    angle: f64,
    last_raw: f64,
    both_faces: bool,
    /// Value shown next to the cursor (studs, degrees, size delta or scale factor).
    pub shown: f64,
    lock: Option<Lock>,
    motion: Rigid,
    body: Option<Body>,
    typed: Option<String>,
    /// The point snap target this frame, if any (for its glyph and the status line).
    pub snap: Option<SnapCandidate>,
    /// The surface a body drag is on this frame (for the grid patch).
    pub surface: Option<Surface>,
    pending: Option<Command>,
    models: Vec<(InstanceId, Pivot)>,
    rotates: bool,
    resizes: bool,
    names: [PropName; 6],
    buf: Vec<SnapCandidate>,
    /// The last frame Collisions accepted: transforms, motion and shown value.
    held: (Vec<PartTransform>, Rigid, f64),
}

fn read_parts(world: &World, ids: &[InstanceId]) -> Vec<DragPart> {
    ids.iter()
        .filter_map(|&id| {
            let start = PartTransform::read(world, id)?;
            let start_orientation = match world.get_ref(id, "Orientation") {
                Ok(Value::Vector3(v)) => *v,
                _ => return None,
            };
            let ball = matches!(world.get_ref(id, "Shape"), Ok(Value::String(s)) if s == "Ball");
            Some(DragPart {
                id,
                start,
                start_orientation,
                ball,
            })
        })
        .collect()
}

impl GizmoDrag {
    fn empty(frame: GizmoFrame, handle: Handle, ray: &Ray, parts: Vec<DragPart>) -> GizmoDrag {
        GizmoDrag {
            handle,
            frame,
            about: frame.origin,
            target: Target::Parts,
            current: parts.iter().map(|p| p.start).collect(),
            ids: parts.iter().map(|p| p.id).collect(),
            parts,
            start_ray: *ray,
            free_normal: -ray.dir,
            start_param: 0.0,
            start_point: DVec3::ZERO,
            angle: 0.0,
            last_raw: 0.0,
            both_faces: false,
            shown: 0.0,
            lock: None,
            motion: Rigid::IDENTITY,
            body: None,
            typed: None,
            snap: None,
            surface: None,
            pending: None,
            models: Vec::new(),
            rotates: false,
            resizes: false,
            names: [
                "Position".into(),
                "Orientation".into(),
                "Size".into(),
                "PivotOffset".into(),
                "WorldPivot".into(),
                "WorldPivotSet".into(),
            ],
            buf: Vec::new(),
            held: (Vec::new(), Rigid::IDENTITY, 0.0),
        }
    }

    /// Starts a drag of `handle` from the pointer `ray`; `None` when the ray does not meet the
    /// handle's line or plane (looking straight down an arrow) or no part can move.
    pub fn begin(
        world: &World,
        frame: GizmoFrame,
        handle: Handle,
        ray: &Ray,
        parts: &[InstanceId],
    ) -> Option<GizmoDrag> {
        let parts = read_parts(world, parts);
        if parts.is_empty() {
            return None;
        }
        let mut drag = GizmoDrag::empty(frame, handle, ray, parts);
        drag.init_handle(ray)?;
        Some(drag)
    }

    fn init_handle(&mut self, ray: &Ray) -> Option<()> {
        let f = self.frame;
        match self.handle {
            Handle::Axis(a) | Handle::Face(a, _) => {
                self.start_param = ray.closest_on_line(f.origin, f.axes[a])?;
            }
            Handle::Uniform => {
                let diag = (f.axes[0] + f.axes[1] + f.axes[2]).normalize_or(DVec3::Y);
                self.start_param = ray.closest_on_line(f.origin, diag)?;
                if self.start_param <= 1e-9 {
                    return None;
                }
            }
            Handle::Plane(a) | Handle::Ring(a) => {
                let t = ray.plane_hit(f.origin, f.axes[a])?;
                self.start_point = ray.at(t);
            }
            Handle::Free => {
                let t = ray.plane_hit(f.origin, self.free_normal)?;
                self.start_point = ray.at(t);
            }
            Handle::Body => {}
        }
        Some(())
    }

    /// Starts dragging the parts themselves (spec 4.1, 5.2). `grab` is the world point under the
    /// cursor on the selection (with V: the selection's vertex nearest the cursor); `active` is
    /// the part whose up axis "Align to surface" turns to the surface normal.
    pub fn begin_body(
        world: &World,
        frame: GizmoFrame,
        ray: &Ray,
        parts: &[InstanceId],
        grab: DVec3,
        active: Option<InstanceId>,
    ) -> Option<GizmoDrag> {
        let parts = read_parts(world, parts);
        if parts.is_empty() {
            return None;
        }
        let mut corners = Vec::with_capacity(parts.len() * 8);
        for p in &parts {
            corners.extend_from_slice(&p.start.corners());
        }
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for c in &corners {
            lo = lo.min(*c);
            hi = hi.max(*c);
        }
        let r0 = (lo + hi) * 0.5;
        for c in corners.iter_mut() {
            *c -= r0;
        }
        let up = active
            .and_then(|a| parts.iter().find(|p| p.id == a))
            .unwrap_or(&parts[0])
            .start
            .axis(1);
        let mut drag = GizmoDrag::empty(frame, Handle::Body, ray, parts);
        drag.body = Some(Body {
            r0,
            rotated: Vec::with_capacity(corners.len()),
            corners,
            grab: grab - r0,
            up,
            user: DQuat::IDENTITY,
            last: None,
            ground: lo.y,
        });
        Some(drag)
    }

    /// Starts moving or turning only the pivot of `id` (pivot edit mode). `start` is its world
    /// pivot and `points` the 27 snap points of its box.
    pub fn begin_pivot(
        frame: GizmoFrame,
        handle: Handle,
        ray: &Ray,
        id: InstanceId,
        name: String,
        start: Pivot,
        points: [DVec3; 27],
    ) -> Option<GizmoDrag> {
        if !matches!(
            handle,
            Handle::Axis(_) | Handle::Plane(_) | Handle::Free | Handle::Ring(_)
        ) {
            return None;
        }
        let mut drag = GizmoDrag::empty(frame, handle, ray, Vec::new());
        drag.target = Target::Pivot {
            id,
            name,
            start,
            points: Box::new(points),
        };
        drag.init_handle(ray)?;
        Some(drag)
    }

    /// Models in the drag whose stored pivots move with it (spec 4.4, `PivotTo` semantics).
    pub fn with_models(mut self, models: Vec<(InstanceId, Pivot)>) -> GizmoDrag {
        self.models = models;
        self
    }

    /// A command already applied for this gesture (Alt-drag's duplicate); it becomes part of the
    /// one undo step, or is undone by [`GizmoDrag::cancel`].
    pub fn with_pending(mut self, cmd: Command) -> GizmoDrag {
        self.pending = Some(cmd);
        self
    }

    /// Turns and scales happen about `p` instead of the gizmo origin (the gizmo was brought to
    /// the cursor with Space, but the pivot stays where it is).
    pub fn with_about(mut self, p: DVec3) -> GizmoDrag {
        self.about = p;
        self
    }

    /// Normal of the screen plane for the centre disc (towards the camera).
    pub fn with_screen_normal(mut self, n: DVec3) -> GizmoDrag {
        self.free_normal = n.normalize_or(DVec3::Z);
        if self.handle == Handle::Free {
            let t = self
                .start_ray
                .plane_hit(self.frame.origin, self.free_normal)
                .unwrap_or(0.0);
            self.start_point = self.start_ray.at(t);
        }
        self
    }

    /// The pending command (Alt-drag's duplicate), to re-apply when amending the step.
    pub fn pending_mut(&mut self) -> Option<&mut Command> {
        self.pending.as_mut()
    }

    /// True when the drag has not moved, turned or resized anything yet.
    pub fn is_unmoved(&self) -> bool {
        self.motion.is_identity() && !self.resizes
    }

    /// True when the drag carries a pending command (Alt-drag's duplicate).
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// This frame's transforms of the moved parts.
    pub fn current(&self) -> &[PartTransform] {
        &self.current
    }

    /// The ids of the moved parts.
    pub fn moving(&self) -> &HashSet<InstanceId> {
        &self.ids
    }

    /// Collisions (spec 3.1), after [`GizmoDrag::update`]: with `blocked` the drag goes back to the
    /// last frame that was accepted (the start at first); otherwise this frame is accepted. No
    /// allocation once the buffer has grown.
    pub fn hold_if(&mut self, blocked: bool) {
        let (held, motion, shown) = &mut self.held;
        if blocked {
            if held.len() == self.current.len() {
                self.current.copy_from_slice(held);
                self.motion = *motion;
                self.shown = *shown;
            } else {
                for (c, p) in self.current.iter_mut().zip(&self.parts) {
                    *c = p.start;
                }
                self.motion = Rigid::IDENTITY;
                self.shown = 0.0;
            }
        } else {
            held.clear();
            held.extend_from_slice(&self.current);
            *motion = self.motion;
            *shown = self.shown;
        }
    }

    /// The parts this drag moves.
    pub fn part_ids(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.parts.iter().map(|p| p.id)
    }

    /// True for a pivot edit drag.
    pub fn is_pivot(&self) -> bool {
        matches!(self.target, Target::Pivot { .. })
    }

    /// Locks (or with the same lock again, unlocks) movement to a world axis or plane. Rotate,
    /// resize and scale drags ignore locks.
    pub fn lock(&mut self, lock: Lock) {
        if matches!(
            self.handle,
            Handle::Ring(_) | Handle::Face(..) | Handle::Uniform
        ) {
            return;
        }
        self.lock = if self.lock == Some(lock) {
            None
        } else {
            Some(lock)
        };
    }

    /// The active lock.
    pub fn current_lock(&self) -> Option<Lock> {
        self.lock
    }

    /// Turns a body drag 90 degrees about `axis` (R: the surface normal; T: the camera's right).
    pub fn turn(&mut self, axis: DVec3) {
        if let Some(b) = &mut self.body {
            let q =
                DQuat::from_axis_angle(axis.normalize_or(DVec3::Y), std::f64::consts::FRAC_PI_2);
            b.user = (q * b.user).normalize();
        }
    }

    /// The normal R turns about: the surface under the body drag, else up.
    pub fn turn_axis(&self) -> DVec3 {
        self.surface.map_or(DVec3::Y, |s| s.normal)
    }

    /// True when typing a number makes sense (one-axis amounts: arrows, rings, faces, corner, or
    /// any move locked to an axis).
    pub fn accepts_typing(&self) -> bool {
        matches!(self.lock, Some(Lock::Axis(_)))
            || matches!(
                self.handle,
                Handle::Axis(_) | Handle::Ring(_) | Handle::Face(..) | Handle::Uniform
            )
    }

    /// Adds a typed character (digits, `.`, `-`, `+`, `*`, `/`, brackets). While text is typed
    /// the pointer no longer moves the selection.
    pub fn type_char(&mut self, c: char) -> bool {
        if !self.accepts_typing() || !(c.is_ascii_digit() || "+-*/.()% ".contains(c)) {
            return false;
        }
        let t = self.typed.get_or_insert_with(String::new);
        if t.len() < crate::expr::MAX_LEN {
            t.push(c);
        }
        true
    }

    /// Drops the typed text (release without Enter: the pointer value wins).
    pub fn clear_typed(&mut self) {
        self.typed = None;
    }

    /// Where the grab point is now (a body drag's grab, else the pivot), for stamping.
    pub fn grab_now(&self) -> DVec3 {
        self.motion.point(self.grab_point())
    }

    /// Removes the last typed character; typing ends when the text is empty.
    pub fn backspace(&mut self) {
        if let Some(t) = &mut self.typed {
            t.pop();
            if t.is_empty() {
                self.typed = None;
            }
        }
    }

    /// The typed text, if typing.
    pub fn typed(&self) -> Option<&str> {
        self.typed.as_deref()
    }

    /// Applies the typed value (Enter). False when nothing valid is typed.
    pub fn commit_typed(&mut self) -> bool {
        let Some(v) = self.typed.as_deref().and_then(crate::expr::eval) else {
            return false;
        };
        self.set_amount(v)
    }

    fn axis_name(&self) -> &'static str {
        let a = match (self.lock, self.handle) {
            (Some(Lock::Axis(a) | Lock::Plane(a)), _) => a,
            (_, Handle::Axis(a) | Handle::Plane(a) | Handle::Ring(a) | Handle::Face(a, _)) => a,
            _ => return "",
        };
        ["X", "Y", "Z"][a]
    }

    /// Sets the exact amount on the dragged axis: studs for arrows and axis locks, degrees for
    /// rings, size change for faces, factor for the corner. False for drags without one axis.
    pub fn set_amount(&mut self, v: f64) -> bool {
        let f = self.frame;
        match (self.lock, self.handle) {
            (Some(Lock::Axis(a)), _) => {
                self.set_motion(Rigid::translate(DVec3::AXES[a] * v));
            }
            (_, Handle::Axis(a)) => self.set_motion(Rigid::translate(f.axes[a] * v)),
            (_, Handle::Ring(a)) => self.set_rotation(a, v),
            (_, Handle::Face(a, positive)) => self.set_resize(a, positive, v),
            (_, Handle::Uniform) => self.set_uniform(v),
            _ => return false,
        }
        self.shown = v;
        true
    }

    fn set_motion(&mut self, rigid: Rigid) {
        self.motion = rigid;
        if !rigid.rotation.abs_diff_eq(DQuat::IDENTITY, 1e-12) {
            self.rotates = true;
        }
        for (cur, p) in self.current.iter_mut().zip(&self.parts) {
            cur.position = rigid.point(p.start.position);
            cur.rotation = (rigid.rotation * p.start.rotation).normalize();
            cur.size = p.start.size;
        }
    }

    fn set_rotation(&mut self, axis: usize, deg: f64) {
        let f = self.frame;
        self.set_motion(Rigid {
            rotation: DQuat::from_axis_angle(f.axes[axis], deg.to_radians()),
            about: self.about,
            translation: DVec3::ZERO,
        });
    }

    fn set_resize(&mut self, axis: usize, positive: bool, d: f64) {
        self.resizes = true;
        let sign = if positive { 1.0 } else { -1.0 };
        let dir = self.frame.axes[axis];
        let both = self.both_faces;
        for (cur, p) in self.current.iter_mut().zip(&self.parts) {
            *cur = resize_face(p, dir, sign, d, both);
        }
    }

    fn min_factor(&self) -> f64 {
        let smallest = self
            .parts
            .iter()
            .map(|p| p.start.size.min_element())
            .fold(f64::INFINITY, f64::min);
        if smallest.is_finite() && smallest > 0.0 {
            MIN_SIZE / smallest
        } else {
            0.0
        }
    }

    fn set_uniform(&mut self, factor: f64) {
        self.resizes = true;
        let largest = self
            .parts
            .iter()
            .map(|p| p.start.size.max_element())
            .fold(0.0, f64::max);
        let max_factor = if largest > 0.0 {
            MAX_PART_SIZE / largest
        } else {
            f64::INFINITY
        };
        // Uniform: the factor stops where the smallest axis hits the floor or the largest hits
        // core's size limit.
        let f = factor.max(self.min_factor()).min(max_factor);
        let o = self.about;
        for (cur, p) in self.current.iter_mut().zip(&self.parts) {
            cur.position = o + (p.start.position - o) * f;
            cur.size = p.start.size * f;
            cur.rotation = p.start.rotation;
        }
        self.shown = f;
    }

    fn constraint(&self) -> Option<Constraint> {
        let f = self.frame;
        let plane = |n: DVec3, u: DVec3, v: DVec3| Constraint::Plane(n, u, v);
        Some(match (self.lock, self.handle) {
            (Some(Lock::Axis(a)), _) => Constraint::Line(DVec3::AXES[a]),
            (Some(Lock::Plane(a)), _) => plane(
                DVec3::AXES[a],
                DVec3::AXES[(a + 1) % 3],
                DVec3::AXES[(a + 2) % 3],
            ),
            (None, Handle::Axis(a)) => Constraint::Line(f.axes[a]),
            (None, Handle::Plane(a)) => plane(f.axes[a], f.axes[(a + 1) % 3], f.axes[(a + 2) % 3]),
            (None, Handle::Free) => Constraint::Free(self.free_normal),
            _ => return None,
        })
    }

    /// The point that moves with the pointer: the pivot for handles, the grab point for a body.
    fn grab_point(&self) -> DVec3 {
        match &self.body {
            Some(b) => b.r0 + b.grab,
            None => match &self.target {
                Target::Pivot { start, .. } => start.position,
                Target::Parts => self.frame.origin,
            },
        }
    }

    fn find_snap(&mut self, input: &DragInput<'_>) -> Option<SnapCandidate> {
        let (view, picks) = (input.view?, input.picks?);
        let (kinds, radius) = SnapKinds::for_settings(input.snapping, input.vertex);
        if kinds.is_empty() {
            return None;
        }
        let ids = &self.ids;
        let skip = |id: InstanceId| ids.contains(&id);
        let q = SnapQuery {
            view,
            cursor: input.cursor,
            radius_px: radius,
            kinds,
            skip: &skip,
        };
        let sources: [&dyn SnapSource; 1] = [picks];
        snapping::find(&sources, &q, &mut self.buf)
    }

    /// The nearest of the 27 pivot points to the cursor, within the snap radius.
    fn pivot_point(&self, input: &DragInput<'_>) -> Option<SnapCandidate> {
        let Target::Pivot { points, .. } = &self.target else {
            return None;
        };
        let view = input.view?;
        points
            .iter()
            .filter_map(|&p| {
                let s = view.project(p)?;
                let d = dist2(s, input.cursor);
                (d <= SNAP_RADIUS_PX).then_some(SnapCandidate {
                    point: p,
                    kind: SnapKind::Vertex,
                    owner: None,
                    screen_dist: d,
                })
            })
            .min_by(|a, b| a.screen_dist.total_cmp(&b.screen_dist))
    }

    /// The transforms for this frame's pointer.
    pub fn update(&mut self, input: &DragInput<'_>) -> &[PartTransform] {
        if self.typed.is_some() {
            return &self.current;
        }
        self.snap = None;
        self.surface = None;
        self.both_faces = input.mods.alt;
        let s = input.snapping;
        let snap_on = s.grid != input.mods.invert_snap;
        let speed = if input.mods.precision { 0.1 } else { 1.0 };
        let f = self.frame;
        let ray = input.ray;
        match (self.handle, self.lock) {
            (Handle::Ring(a), _) => {
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
                    deg = snap(deg, s.rotate_step);
                }
                self.shown = deg;
                self.set_rotation(a, deg);
            }
            (Handle::Face(a, positive), _) => {
                let Some(p) = ray.closest_on_line(f.origin, f.axes[a]) else {
                    return &self.current;
                };
                let sign = if positive { 1.0 } else { -1.0 };
                let mut d = (p - self.start_param) * sign * speed;
                if snap_on {
                    d = snap(d, s.move_step);
                }
                self.shown = d;
                self.set_resize(a, positive, d);
            }
            (Handle::Uniform, _) => {
                let diag = (f.axes[0] + f.axes[1] + f.axes[2]).normalize_or(DVec3::Y);
                let Some(p) = ray.closest_on_line(f.origin, diag) else {
                    return &self.current;
                };
                let mut factor = 1.0 + (p / self.start_param - 1.0) * speed;
                if snap_on {
                    // Snap the selection's largest size to the increment.
                    let largest = self
                        .parts
                        .iter()
                        .map(|p| p.start.size.max_element())
                        .fold(0.0, f64::max);
                    if largest > 0.0 {
                        factor = snap(largest * factor, s.move_step) / largest;
                    }
                }
                self.set_uniform(factor);
            }
            (Handle::Body, None) => self.update_body(input, snap_on),
            _ => self.update_move(input, snap_on, speed),
        }
        &self.current
    }

    fn update_move(&mut self, input: &DragInput<'_>, snap_on: bool, speed: f64) {
        let Some(c) = self.constraint() else { return };
        let ray = input.ray;
        let origin = self.grab_point();
        let step = input.snapping.move_step;
        let mut delta = match c {
            Constraint::Line(dir) => {
                let (Some(s), Some(s0)) = (
                    ray.closest_on_line(origin, dir),
                    self.start_ray.closest_on_line(origin, dir),
                ) else {
                    return;
                };
                let mut d = (s - s0) * speed;
                if snap_on {
                    d = snap(d, step);
                }
                self.shown = d;
                dir * d
            }
            Constraint::Plane(n, u, v) => {
                let (Some(t), Some(t0)) = (
                    ray.plane_hit(origin, n),
                    self.start_ray.plane_hit(origin, n),
                ) else {
                    return;
                };
                let d = (ray.at(t) - self.start_ray.at(t0)) * speed;
                let (mut du, mut dv) = (d.dot(u), d.dot(v));
                if snap_on {
                    du = snap(du, step);
                    dv = snap(dv, step);
                }
                self.shown = (du * du + dv * dv).sqrt();
                u * du + v * dv
            }
            Constraint::Free(n) => {
                let (Some(t), Some(t0)) = (
                    ray.plane_hit(origin, n),
                    self.start_ray.plane_hit(origin, n),
                ) else {
                    return;
                };
                let mut d = (ray.at(t) - self.start_ray.at(t0)) * speed;
                if snap_on {
                    d = DVec3::new(snap(d.x, step), snap(d.y, step), snap(d.z, step));
                }
                self.shown = d.length();
                d
            }
        };
        // Point snapping: the grab point lands exactly on the target, within the constraint.
        let target = self.pivot_point(input).or_else(|| self.find_snap(input));
        if let Some(t) = target {
            let want = t.point - origin;
            delta = match c {
                Constraint::Line(dir) => dir * want.dot(dir),
                Constraint::Plane(n, ..) if self.lock.is_some() => want - n * want.dot(n),
                // The plane squares and the disc follow the target in 3D.
                _ => want,
            };
            self.shown = delta.length();
            self.snap = Some(t);
        }
        self.set_motion(Rigid::translate(delta));
    }

    fn update_body(&mut self, input: &DragInput<'_>, snap_on: bool) {
        // Point snapping first: it needs the scratch buffer, the body is borrowed below.
        let point = if input.onto_surface {
            None
        } else {
            self.find_snap(input)
        };
        let Some(mut b) = self.body.take() else {
            return;
        };
        let ray = input.ray;
        let s = input.snapping;
        let ids = &self.ids;
        let skip = |id: InstanceId| ids.contains(&id);
        let hit = if s.surface {
            input.picks.and_then(|p| {
                let (h, normal) = p.surface(&ray, &skip)?;
                let t = p.transform(h.id)?;
                Some(Surface {
                    point: ray.at(h.distance),
                    normal,
                    origin: t.position,
                    rotation: t.rotation,
                    part: Some(h.id),
                })
            })
        } else {
            None
        };
        let surf = hit
            .or_else(|| b.last.and_then(|l| l.extend(&ray)))
            .or_else(|| Surface::ground(b.ground).extend(&ray));
        let Some(surf) = surf else {
            self.body = Some(b);
            return;
        };
        if surf.part.is_some() {
            b.last = Some(surf);
        }
        let mut q = b.user;
        if s.align {
            let up = (q * b.up).normalize_or(DVec3::Y);
            q = (DQuat::from_rotation_arc(up, surf.normal) * q).normalize();
        }
        b.rotated.clear();
        b.rotated.extend(b.corners.iter().map(|c| q * *c));
        let grab = q * b.grab;
        let step = snap_on.then_some(s.move_step);
        let mut target = None;
        let r = if input.onto_surface {
            surf.point - grab
        } else {
            target = point;
            if target.is_none() && input.vertex {
                target = nearest_corner(input, &surf);
            }
            match target {
                Some(t) => t.point - grab,
                None => place_on_surface(&b.rotated, grab, &surf, step),
            }
        };
        self.snap = target.or(Some(SnapCandidate {
            point: surf.point,
            kind: if step.is_some() {
                SnapKind::Grid
            } else {
                SnapKind::Surface
            },
            owner: surf.part,
            screen_dist: 0.0,
        }));
        self.surface = Some(surf);
        self.shown = (r - b.r0).length();
        let rigid = Rigid {
            rotation: q,
            about: b.r0,
            translation: r - b.r0,
        };
        self.body = Some(b);
        self.set_motion(rigid);
    }

    /// Writes the current transforms through `overlay` (live preview). A move writes only
    /// `Position`; `Orientation` and `Size` are written once the drag turns or resizes.
    pub fn apply(&self, world: &mut World, overlay: &mut PreviewOverlay) {
        let unrotated = self.motion.rotation.abs_diff_eq(DQuat::IDENTITY, 1e-12);
        for (cur, p) in self.current.iter().zip(&self.parts) {
            let _ = overlay.set(
                world,
                p.id,
                &self.names[POSITION],
                Value::Vector3(from_d(cur.position)),
            );
            if self.rotates {
                let o = if unrotated {
                    p.start_orientation
                } else {
                    quat_to_orientation(cur.rotation)
                };
                let _ = overlay.set(world, p.id, &self.names[ORIENTATION], Value::Vector3(o));
            }
            if self.resizes {
                let _ = overlay.set(
                    world,
                    p.id,
                    &self.names[SIZE],
                    Value::Vector3(from_d(cur.size)),
                );
            }
        }
        // Pivots are core properties: a pivot edit and the WorldPivots of moved Models are
        // written through the overlay too, so they land in the same one step. Every frame writes
        // them; back at no motion (dragged out and back, typed 0) they return to the values the
        // gesture started from, exactly, so nothing stale is committed.
        let identity = self.motion.is_identity();
        if let Target::Pivot { id, start, .. } = &self.target {
            for (target, prop, value) in pivot_writes(world, *id, &self.motion.frame(start)) {
                write_or_restore(
                    world,
                    overlay,
                    target,
                    &self.prop_name(prop),
                    value,
                    identity,
                );
            }
        }
        for (id, start) in &self.models {
            let value = Value::CFrame(Box::new(to_cframe(&self.motion.frame(start))));
            write_or_restore(
                world,
                overlay,
                *id,
                &self.prop_name("WorldPivot"),
                value,
                identity,
            );
        }
    }

    fn prop_name(&self, prop: &str) -> PropName {
        self.names
            .iter()
            .find(|n| &***n == prop)
            .cloned()
            .unwrap_or_else(|| prop.into())
    }

    /// Esc: restores every previewed value. Returns the pending command (Alt-drag's duplicate)
    /// for the caller to pass to `History::abandon`.
    pub fn cancel(self, world: &mut World, overlay: &mut PreviewOverlay) -> Option<Command> {
        overlay.cancel(world);
        self.pending
    }

    /// Undo name of the gesture.
    pub fn name(&self) -> String {
        if let Target::Pivot { name, .. } = &self.target {
            return format!("Edit pivot of {name}");
        }
        if self.pending.is_some() {
            return "Duplicate and move".into();
        }
        let verb = match self.handle {
            Handle::Ring(_) => "Rotate",
            Handle::Face(..) => "Resize",
            Handle::Uniform => "Scale",
            _ => "Move",
        };
        format!("{verb} {}", count(self.parts.len(), "part"))
    }

    /// The pivot a pivot drag has reached.
    pub fn pivot_now(&self) -> Option<Pivot> {
        match &self.target {
            Target::Pivot { start, .. } => Some(self.motion.frame(start)),
            Target::Parts => None,
        }
    }

    /// Ends the drag. Returns (the one undo step to record, a pending command to abandon when
    /// nothing changed). Everything the drag wrote, pivots included, is in the overlay's one
    /// `SetProperties`; an Alt-drag's duplicate comes first in a batch.
    pub fn finish(self, overlay: &mut PreviewOverlay) -> (Option<Command>, Option<Command>) {
        let name = self.name();
        let Some(props) = overlay.commit(name.clone()) else {
            return (None, self.pending);
        };
        match self.pending {
            Some(p) => (
                Some(Command::Batch {
                    name,
                    commands: vec![p, props],
                }),
                None,
            ),
            None => (Some(props), None),
        }
    }

    /// Text next to the cursor (the typed field while typing).
    pub fn label(&self) -> String {
        if let Some(t) = &self.typed {
            return format!("{}: {t}", self.axis_name());
        }
        let axis = self.axis_name();
        match (self.lock, self.handle) {
            (Some(Lock::Axis(_)), _) | (None, Handle::Axis(_)) => {
                format!("{axis} {:+.2}", self.shown)
            }
            (_, Handle::Ring(_)) => format!("{axis} {:+.1}°", self.shown),
            (_, Handle::Face(..)) => format!("Size {axis} {:+.2}", self.shown),
            (_, Handle::Uniform) => format!("x{:.2}", self.shown),
            (Some(Lock::Plane(_)), _) => format!("{axis} plane {:.2} studs", self.shown),
            _ => format!("{:.2} studs", self.shown),
        }
    }

    /// The frame the drag started with.
    pub fn frame(&self) -> GizmoFrame {
        self.frame
    }

    /// Where to draw the gizmo now: moved with the selection for moves and pivot edits, where it
    /// was grabbed for rotate and resize.
    pub fn shown_frame(&self) -> GizmoFrame {
        let mut f = self.frame;
        match self.handle {
            Handle::Ring(_) | Handle::Face(..) | Handle::Uniform => {
                if self.is_pivot() {
                    for a in f.axes.iter_mut() {
                        *a = self.motion.rotation * *a;
                    }
                }
            }
            _ => f.origin = self.motion.point(f.origin),
        }
        f
    }

    /// The motion so far (identity for resizes).
    pub fn motion(&self) -> Rigid {
        self.motion
    }
}

/// With V held over a part: that part's corner nearest the cursor.
fn nearest_corner(input: &DragInput<'_>, surf: &Surface) -> Option<SnapCandidate> {
    let (view, picks, part) = (input.view?, input.picks?, surf.part?);
    let t = picks.transform(part)?;
    t.corners()
        .iter()
        .filter_map(|&c| {
            let s = view.project(c)?;
            Some(SnapCandidate {
                point: c,
                kind: SnapKind::Vertex,
                owner: Some(part),
                screen_dist: dist2(s, input.cursor),
            })
        })
        .min_by(|a, b| a.screen_dist.total_cmp(&b.screen_dist))
}

/// Resizes one face of `p` along the gizmo axis `axis` (world space): the face on side `sign`
/// moves by `d`, the opposite face stays put (both faces move with `both`). The part's local
/// axis closest to `axis` is resized.
fn resize_face(p: &DragPart, axis: DVec3, sign: f64, d: f64, both: bool) -> PartTransform {
    let mut t = p.start;
    let local = (0..3)
        .max_by(|&x, &y| {
            t.axis(x)
                .dot(axis)
                .abs()
                .total_cmp(&t.axis(y).dot(axis).abs())
        })
        .unwrap_or(0);
    let grow = if both { 2.0 * d } else { d };
    let shift = |start: f64, new: f64| {
        if both {
            DVec3::ZERO
        } else {
            axis * (sign * (new - start) * 0.5)
        }
    };
    if p.ball {
        let start = p.start.size.min_element();
        let new = (start + grow).clamp(MIN_SIZE, MAX_PART_SIZE);
        t.size = DVec3::splat(new);
        t.position = p.start.position + shift(start, new);
        return t;
    }
    let start = p.start.size[local];
    let new = (start + grow).clamp(MIN_SIZE, MAX_PART_SIZE);
    t.size[local] = new;
    t.position = p.start.position + shift(start, new);
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
    if frame.has_rotate() {
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
    if frame.has_move() {
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
        let disc = if active == Some(Handle::Free) {
            DISC_PX as f32 + 1.5
        } else {
            DISC_PX as f32
        };
        painter.circle(
            o,
            disc,
            ui_tokens().icon,
            Stroke::new(1.0, ui_tokens().icon_hole),
        );
    }
    if frame.has_scale() {
        for axis in 0..3 {
            let c = axis_colour(axis);
            for positive in [true, false] {
                let h = Handle::Face(axis, positive);
                let Some(p) = to(frame.face_point(axis, positive)) else {
                    continue;
                };
                if frame.tool == Tool::Scale {
                    painter.line_segment([o, p], Stroke::new(1.5, c.gamma_multiply(0.7)));
                }
                let half = if active == Some(h) { 6.0 } else { 4.5 };
                painter.rect_filled(
                    egui::Rect::from_center_size(p, egui::vec2(half, half) * 2.0),
                    1.0,
                    c,
                );
            }
        }
        if let Some(p) = to(frame.corner_point()) {
            let half = if active == Some(Handle::Uniform) {
                CORNER_PX as f32 + 1.5
            } else {
                CORNER_PX as f32
            };
            painter.rect(
                egui::Rect::from_center_size(p, egui::vec2(half, half) * 2.0),
                1.0,
                ui_tokens().icon,
                Stroke::new(1.0, ui_tokens().icon_hole),
                egui::StrokeKind::Outside,
            );
        }
    }
    if let (Some(d), Some(p)) = (drag, pointer) {
        label_box(
            painter,
            p + egui::vec2(16.0, 12.0),
            &d.label(),
            d.typed().is_some(),
        );
    }
}

/// A small mono label box at `at` (the drag value, or the typed field with an accent ring).
pub fn label_box(painter: &egui::Painter, at: Pos2, text: &str, editing: bool) {
    let t = ui_tokens();
    let galley = painter.layout_no_wrap(text.to_string(), t.mono(), t.text);
    let rect = egui::Rect::from_min_size(at, galley.size() + egui::vec2(10.0, 6.0));
    painter.rect_filled(rect, 4.0, t.panel);
    if editing {
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    painter.galley(rect.min + egui::vec2(5.0, 3.0), galley, t.text);
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
    let v0 = v0 / len * frame.ring_radius();
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

/// Shows `value` for `prop` of `id`; with `restore`, puts back the value the gesture started from
/// instead (and writes nothing when the gesture has not touched it).
fn write_or_restore(
    world: &mut World,
    overlay: &mut PreviewOverlay,
    id: InstanceId,
    prop: &PropName,
    value: Value,
    restore: bool,
) {
    if !restore {
        let _ = overlay.set(world, id, prop, value);
    } else if let Some(original) = overlay.original_named(id, prop).cloned() {
        let _ = overlay.set(world, id, prop, original);
    }
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

    fn input(ray: Ray, s: &Snapping, mods: DragModifiers) -> DragInput<'_> {
        DragInput::simple(ray, s, mods)
    }

    #[test]
    fn axis_drag_snaps_relative_to_start() {
        let (mut w, p) = world_with_part(Vec3::new(0.3, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::new(0.3, 0.0, 0.0), Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(0), &ray_down(0.3, 0.0), &[p]).unwrap();
        let snapping = Snapping::default();
        let mut o = PreviewOverlay::new();
        d.update(&input(
            ray_down(1.6, 0.0),
            &snapping,
            DragModifiers::default(),
        ));
        d.apply(&mut w, &mut o);
        let Value::Vector3(pos) = w.get(p, "Position").unwrap() else {
            panic!()
        };
        assert!(
            (pos.x - 1.3).abs() < 1e-9,
            "0.3 + 1 step = 1.3, got {}",
            pos.x
        );
        let ctrl = DragModifiers {
            invert_snap: true,
            ..DragModifiers::default()
        };
        d.update(&input(ray_down(1.6, 0.0), &snapping, ctrl));
        assert!((d.current[0].position.x - 1.6).abs() < 1e-9);
        let fine = DragModifiers {
            precision: true,
            ..ctrl
        };
        d.update(&input(ray_down(10.3, 0.0), &snapping, fine));
        assert!((d.current[0].position.x - 1.3).abs() < 1e-9);
    }

    #[test]
    fn finish_is_one_step_and_cancel_restores() {
        let (mut w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut o = PreviewOverlay::new();
        let start =
            |w: &World| GizmoDrag::begin(w, f, Handle::Axis(2), &ray_down(0.0, 0.0), &[p]).unwrap();
        let mut d = start(&w);
        let s = Snapping::default();
        d.update(&input(ray_down(0.0, 3.0), &s, DragModifiers::default()));
        d.apply(&mut w, &mut o);
        assert!(d.cancel(&mut w, &mut o).is_none());
        assert_eq!(
            w.get(p, "Position").unwrap(),
            Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
        );
        let mut d = start(&w);
        d.update(&input(ray_down(0.0, 3.0), &s, DragModifiers::default()));
        d.apply(&mut w, &mut o);
        let (cmd, abandon) = d.finish(&mut o);
        let cmd = cmd.unwrap();
        assert!(abandon.is_none());
        assert_eq!(cmd.name(&w), "Move 1 part");
        let Command::SetProperties { changes, .. } = cmd else {
            panic!("{cmd:?}")
        };
        assert_eq!(changes.len(), 1, "a move writes Position only");
    }

    #[test]
    fn move_keeps_an_unrounded_orientation_untouched() {
        let (mut w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let odd = Vec3::new(12.345678912, 33.3333333333, -7.1);
        w.set(p, "Orientation", Value::Vector3(odd)).unwrap();
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(0), &ray_down(0.0, 0.0), &[p]).unwrap();
        let mut o = PreviewOverlay::new();
        d.update(&input(
            ray_down(2.0, 0.0),
            &Snapping::default(),
            DragModifiers::default(),
        ));
        d.apply(&mut w, &mut o);
        let (Some(Command::SetProperties { changes, .. }), _) = d.finish(&mut o) else {
            panic!()
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(w.get(p, "Orientation").unwrap(), Value::Vector3(odd));
    }

    #[test]
    fn ring_drag_rotates_about_pivot_in_increments() {
        let (w, p) = world_with_part(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let f = frame(DVec3::ZERO, Tool::Rotate);
        let mut d = GizmoDrag::begin(&w, f, Handle::Ring(1), &ray_down(5.0, 0.0), &[p]).unwrap();
        let a = 40f64.to_radians();
        let s = Snapping::default();
        let cur = d.update(&input(
            ray_down(5.0 * a.cos(), -5.0 * a.sin()),
            &s,
            DragModifiers::default(),
        ));
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
            grid: false,
            ..Snapping::default()
        };
        for deg in (10..=270).step_by(10) {
            let a = f64::from(deg).to_radians();
            d.update(&input(
                ray_down(5.0 * a.cos(), -5.0 * a.sin()),
                &s,
                DragModifiers::default(),
            ));
        }
        assert!((d.shown - 270.0).abs() < 1e-6, "{}", d.shown);
    }

    #[test]
    fn face_drag_keeps_opposite_face_floors_size_and_alt_moves_both() {
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0));
        let f = frame(DVec3::ZERO, Tool::Scale);
        let mut d =
            GizmoDrag::begin(&w, f, Handle::Face(0, true), &ray_down(0.0, 0.0), &[p]).unwrap();
        let s = Snapping::default();
        let none = DragModifiers::default();
        let cur = d.update(&input(ray_down(2.0, 0.0), &s, none))[0];
        assert!((cur.size.x - 6.0).abs() < 1e-9);
        assert!((cur.position.x - cur.size.x / 2.0 - -2.0).abs() < 1e-9);
        let cur = d.update(&input(ray_down(-10.0, 0.0), &s, none))[0];
        assert!((cur.size.x - MIN_PART_SIZE).abs() < 1e-9, "core's floor");
        assert!((cur.position.x - cur.size.x / 2.0 - -2.0).abs() < 1e-9);
        let alt = DragModifiers { alt: true, ..none };
        let cur = d.update(&input(ray_down(1.0, 0.0), &s, alt))[0];
        assert!((cur.size.x - 6.0).abs() < 1e-9, "both faces: 4 + 2 * 1");
        assert!(cur.position.x.abs() < 1e-9, "centre stays");
        // Core's largest part size caps the other end.
        let cur = d.update(&input(ray_down(5000.0, 0.0), &s, none))[0];
        assert!((cur.size.x - MAX_PART_SIZE).abs() < 1e-9);
    }

    #[test]
    fn corner_scales_uniformly_about_the_pivot() {
        let (w, p) = world_with_part(Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let f = frame(DVec3::ZERO, Tool::Scale);
        let diag = DVec3::ONE.normalize();
        let along = |s: f64| Ray {
            origin: diag * s + DVec3::new(1.0, -1.0, 0.0).normalize() * 30.0,
            dir: -DVec3::new(1.0, -1.0, 0.0).normalize(),
        };
        let mut d = GizmoDrag::begin(&w, f, Handle::Uniform, &along(4.0), &[p]).unwrap();
        let s = Snapping {
            grid: false,
            ..Snapping::default()
        };
        let cur = d.update(&input(along(6.0), &s, DragModifiers::default()))[0];
        assert!((d.shown - 1.5).abs() < 1e-9, "{}", d.shown);
        assert!((cur.size - DVec3::splat(3.0)).length() < 1e-9);
        assert!((cur.position - DVec3::new(3.0, 0.0, 0.0)).length() < 1e-9);
        assert_eq!(d.name(), "Scale 1 part");
    }

    #[test]
    fn typed_expression_moves_exactly_on_the_dragged_axis() {
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(1), &ray_down(0.0, 0.0), &[p]);
        // Looking straight down the Y arrow cannot grab it; use a side ray.
        assert!(d.is_none());
        let side = |y: f64| Ray {
            origin: DVec3::new(0.0, y, 30.0),
            dir: DVec3::NEG_Z,
        };
        d = GizmoDrag::begin(&w, f, Handle::Axis(1), &side(0.0), &[p]);
        let mut d = d.unwrap();
        for c in "3*4".chars() {
            assert!(d.type_char(c));
        }
        assert!(!d.type_char('x'));
        assert_eq!(d.label(), "Y: 3*4");
        // The pointer no longer moves it while typing.
        d.update(&input(
            side(7.0),
            &Snapping::default(),
            DragModifiers::default(),
        ));
        assert!(d.commit_typed());
        assert!((d.current[0].position - DVec3::new(0.0, 12.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn lock_switches_a_plane_drag_to_one_world_axis() {
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Plane(1), &ray_down(0.5, 0.5), &[p]).unwrap();
        let s = Snapping::default();
        d.update(&input(ray_down(3.5, 2.5), &s, DragModifiers::default()));
        assert!((d.current[0].position - DVec3::new(3.0, 0.0, 2.0)).length() < 1e-9);
        d.lock(Lock::Axis(0));
        d.update(&input(ray_down(3.5, 2.5), &s, DragModifiers::default()));
        assert!((d.current[0].position - DVec3::new(3.0, 0.0, 0.0)).length() < 1e-9);
        assert!(d.accepts_typing());
        d.lock(Lock::Axis(0));
        assert_eq!(d.current_lock(), None, "the same key unlocks");
    }

    #[test]
    fn pivot_drag_snaps_to_box_points_and_finishes_as_one_property_step() {
        let (mut w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 2.0, 2.0));
        let cam = EditorCamera::default();
        let view = cam.view((800.0, 600.0));
        let f = GizmoFrame::new(DVec3::ZERO, DQuat::IDENTITY, Tool::Move, &view);
        let points = crate::pivot::candidates(&w, p).unwrap();
        let grab = view.project(DVec3::ZERO).unwrap();
        let mut d = GizmoDrag::begin_pivot(
            f,
            Handle::Free,
            &view.ray(grab.0, grab.1),
            p,
            "Part".into(),
            Pivot::IDENTITY,
            points,
        )
        .unwrap()
        .with_screen_normal(-cam.forward());
        let corner = DVec3::new(2.0, 1.0, 1.0);
        let c = view.project(corner).unwrap();
        let cursor = (c.0 + 3.0, c.1 + 2.0);
        let s = Snapping::default();
        let mut inp = DragInput::simple(view.ray(cursor.0, cursor.1), &s, DragModifiers::default());
        inp.view = Some(&view);
        inp.cursor = cursor;
        d.update(&inp);
        let now = d.pivot_now().unwrap();
        assert!((now.position - corner).length() < 1e-12);
        let mut o = PreviewOverlay::new();
        d.apply(&mut w, &mut o);
        let (cmd, _) = d.finish(&mut o);
        let Some(Command::SetProperties { changes, name }) = cmd else {
            panic!()
        };
        assert_eq!(name, "Edit pivot of Part");
        assert_eq!(changes.len(), 1);
        assert_eq!(&*changes[0].1, "PivotOffset");
        let now = crate::pivot::world_pivot(&w, p).unwrap();
        assert!((now.position - corner).length() < 1e-9);
    }

    #[test]
    fn hit_finds_arrow_tips_disc_and_corner() {
        let cam = EditorCamera::default();
        let view = cam.view((800.0, 600.0));
        let f = GizmoFrame::new(DVec3::ZERO, DQuat::IDENTITY, Tool::Move, &view);
        let tip = view.project(f.origin + DVec3::Y * f.length * 0.9).unwrap();
        assert_eq!(f.hit(&view, tip), Some(Handle::Axis(1)));
        assert_eq!(f.hit(&view, (5.0, 5.0)), None);
        let corners = f.plane_corners(1);
        let mid = view.project((corners[0] + corners[2]) * 0.5).unwrap();
        assert_eq!(f.hit(&view, mid), Some(Handle::Plane(1)));
        let o = view.project(f.origin).unwrap();
        assert_eq!(f.hit(&view, o), Some(Handle::Free));
        let s = GizmoFrame::new(DVec3::ZERO, DQuat::IDENTITY, Tool::Scale, &view);
        let c = view.project(s.corner_point()).unwrap();
        assert_eq!(s.hit(&view, c), Some(Handle::Uniform));
        let t = GizmoFrame::new(DVec3::ZERO, DQuat::IDENTITY, Tool::Transform, &view);
        let ring = view.project(t.ring_point(1, 3, 64)).unwrap();
        assert_eq!(t.hit(&view, ring), Some(Handle::Ring(1)));
    }

    #[test]
    fn grabbing_a_face_or_corner_of_a_tiny_part_does_not_jump_its_size() {
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.01, 1.0, 1.0));
        let s = Snapping {
            grid: false,
            ..Snapping::default()
        };
        let none = DragModifiers::default();
        let f = frame(DVec3::ZERO, Tool::Scale);
        let mut d =
            GizmoDrag::begin(&w, f, Handle::Face(0, true), &ray_down(0.0, 0.0), &[p]).unwrap();
        let cur = d.update(&input(ray_down(0.0, 0.0), &s, none))[0];
        assert!((cur.size.x - 0.01).abs() < 1e-12, "no jump to a floor");
        let (w, p) = world_with_part(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.001, 4.0, 4.0));
        let diag = DVec3::ONE.normalize();
        let along = |t: f64| Ray {
            origin: diag * t + DVec3::new(1.0, -1.0, 0.0).normalize() * 30.0,
            dir: -DVec3::new(1.0, -1.0, 0.0).normalize(),
        };
        let mut d = GizmoDrag::begin(&w, f, Handle::Uniform, &along(4.0), &[p]).unwrap();
        let cur = d.update(&input(along(4.0), &s, none))[0];
        assert!(
            (cur.size.y - 4.0).abs() < 1e-9,
            "no forced x50: {:?}",
            cur.size
        );
    }

    #[test]
    fn a_set_model_pivot_dragged_out_and_back_is_not_committed_moved() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let m = w.create("Model", Some(ws)).unwrap();
        let p = w.create("Part", Some(m)).unwrap();
        let pivot = Pivot::at(DVec3::new(5.0, 0.0, 0.0));
        for (id, prop, v) in pivot_writes(&w, m, &pivot) {
            w.set(id, prop, v).unwrap();
        }
        let before = w.get(m, "WorldPivot").unwrap();
        let f = frame(DVec3::ZERO, Tool::Move);
        let mut d = GizmoDrag::begin(&w, f, Handle::Axis(0), &ray_down(0.0, 0.0), &[p])
            .unwrap()
            .with_models(vec![(m, pivot)]);
        let s = Snapping {
            grid: false,
            ..Snapping::default()
        };
        let mut o = PreviewOverlay::new();
        d.update(&input(ray_down(3.0, 0.0), &s, DragModifiers::default()));
        d.apply(&mut w, &mut o);
        assert_ne!(w.get(m, "WorldPivot").unwrap(), before, "the pivot follows");
        d.update(&input(ray_down(0.0, 0.0), &s, DragModifiers::default()));
        d.apply(&mut w, &mut o);
        assert_eq!(
            w.get(m, "WorldPivot").unwrap(),
            before,
            "back with the parts"
        );
        let (cmd, _) = d.finish(&mut o);
        assert!(cmd.is_none(), "{cmd:?}");
    }
}
