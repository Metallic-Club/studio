//! The transform tools in the viewport (#355), after Forge-Engine's `tools.rs`: **Select, Move, Scale, Rotate**, in
//! world or local axes. The tool's gizmo sits on the selection: on the active instance's pivot for one, at the middle
//! of everything selected for several. [`super::gizmo`] has its maths; this module runs it.
//!
//! - **Move**: six arrows and three plane handles on the side facing the camera. A drag moves every selected instance
//!   (a model with everything in it, by its pivot) along them, shown with a ruler and the distance.
//!
//! **Drags are measured from where they began**: each frame places the instances from where they were when the drag
//! started and from the pointer, never from the frame before, so nothing adds up rounding. A drag is one undo step:
//! the button is held throughout (see `editor::history`).

use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2};
use metallic_engine::metallic_classes::base::world_object;
use metallic_engine::metallic_core::glam::{Vec2, Vec3};
use metallic_engine::metallic_core::{CFrame, DataModel, Id, PropId};

use super::gizmo::{self, MoveHandle};
use super::pick;
use super::view::{Ray, View};
use crate::editor::edit::is_movable;
use crate::editor::{Scene, Selection};

/// A transform tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Select, and drag parts about (#358).
    #[default]
    Select,
    /// Move along the axes.
    Move,
    /// Resize (#356).
    Scale,
    /// Turn (#357).
    Rotate,
}

impl Tool {
    /// Every tool, in the toolbar's order.
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Move, Tool::Scale, Tool::Rotate];
}

/// A handle of the active tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// One of the Move gizmo's.
    Move(MoveHandle),
}

/// Where the tool's handles are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The gizmo's centre.
    pub origin: Vec3,
    /// Its three axes.
    pub dirs: [Vec3; 3],
    /// How long its handles are.
    pub len: f32,
}

/// An instance a drag moves, as it was when the drag began.
#[derive(Clone, Copy, Debug)]
struct Start {
    id: Id,
    pivot: CFrame,
}

/// A drag in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    handle: Handle,
    frame: Frame,
    starts: Vec<Start>,
    /// Every property the drag can change, which the scene notes for undo.
    watched: Vec<(Id, PropId)>,
    /// Where along the arrow's axis the pointer took hold.
    t0: f32,
    /// Where on the plane it took hold, from the origin.
    grab: Vec3,
    /// How far it has moved along each axis.
    moved: [f32; 3],
}

/// The tool, its axes and what the pointer does with the gizmo: the viewport's part of the editor.
#[derive(Clone, Debug, Default)]
pub struct Gizmo {
    /// The active tool.
    pub tool: Tool,
    /// Whether the handles follow the active instance's axes rather than the world's.
    pub local: bool,
    /// The handle under the pointer.
    hover: Option<Handle>,
    /// The drag in progress.
    drag: Option<Drag>,
}

impl Gizmo {
    /// Whether a handle is being dragged.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
}

/// What the tools act on: the selection's instances that are not inside another selected one and have a pivot.
pub fn targets(dm: &DataModel, selection: &Selection) -> Vec<Id> {
    selection
        .top_level(dm)
        .into_iter()
        .filter(|&id| is_movable(dm, id) && world_object::pivot(dm, id).is_ok())
        .collect()
}

/// Where `tool`'s handles are for `targets` (the active one last), seen from `camera`. `None` for the Select tool and
/// for nothing to act on.
pub fn frame_for(dm: &DataModel, tool: Tool, local: bool, targets: &[Id], camera: Vec3) -> Option<Frame> {
    let &active = targets.last()?;
    if tool == Tool::Select {
        return None;
    }
    let pivot = world_object::pivot(dm, active).ok()?;
    let origin = match targets {
        [_] => pivot.position(),
        _ => {
            let corners: Vec<Vec3> = targets
                .iter()
                .filter_map(|&id| pick::outline_box(dm, id))
                .flat_map(|bounds| bounds.corners())
                .collect();
            let min = corners.iter().copied().reduce(Vec3::min)?;
            let max = corners.iter().copied().reduce(Vec3::max)?;
            (min + max) * 0.5
        }
    };
    Some(Frame {
        origin,
        dirs: gizmo::axis_dirs(local, pivot.rotation()),
        len: gizmo::handle_len(camera, origin),
    })
}

/// The handle of `tool` under `pointer`.
pub fn hit(tool: Tool, frame: &Frame, view: &View, pointer: Pos2) -> Option<Handle> {
    match tool {
        Tool::Move => gizmo::hit_move(view, pointer, frame.origin, &frame.dirs).map(Handle::Move),
        Tool::Select | Tool::Scale | Tool::Rotate => None,
    }
}

/// Every stored `CFrame` in and under `targets`: what moving them can change.
fn watch_list(dm: &DataModel, targets: &[Id]) -> Vec<(Id, PropId)> {
    let mut out = Vec::new();
    for &target in targets {
        let inside = dm.descendants(target).into_iter().flatten();
        for id in std::iter::once(target).chain(inside) {
            if let Ok(prop) = dm.prop_id(id, "CFrame") {
                out.push((id, prop));
            }
        }
    }
    out
}

/// Takes hold of `handle` with the pointer's `ray`.
pub fn begin_drag(dm: &DataModel, handle: Handle, frame: Frame, ray: Ray, targets: &[Id]) -> Option<Drag> {
    let starts = targets
        .iter()
        .filter_map(|&id| world_object::pivot(dm, id).ok().map(|pivot| Start { id, pivot }))
        .collect();
    let mut drag = Drag {
        handle,
        frame,
        starts,
        watched: watch_list(dm, targets),
        t0: 0.0,
        grab: Vec3::ZERO,
        moved: [0.0; 3],
    };
    match handle {
        Handle::Move(MoveHandle::Arrow(axis, _)) => {
            drag.t0 = gizmo::ray_line(ray, frame.origin, frame.dirs[axis])?.1;
        }
        Handle::Move(MoveHandle::Plane(normal)) => {
            let at = gizmo::ray_plane(ray, frame.origin, frame.dirs[normal], gizmo::MIN_PLANE_FACING)?;
            drag.grab = at - frame.origin;
        }
    }
    Some(drag)
}

/// Follows the pointer's `ray`: moves the instances from where they started, as part of the scene's action under
/// way. `step` is the move increment (0: free).
pub fn update_drag(scene: &mut Scene<'_>, drag: &mut Drag, ray: Ray, step: f32) {
    let f = drag.frame;
    let delta = match drag.handle {
        Handle::Move(MoveHandle::Arrow(axis, _)) => {
            let Some((_, t)) = gizmo::ray_line(ray, f.origin, f.dirs[axis]) else {
                return;
            };
            let amount = gizmo::snap(t - drag.t0, step);
            drag.moved = [0.0; 3];
            drag.moved[axis] = amount;
            f.dirs[axis] * amount
        }
        Handle::Move(MoveHandle::Plane(normal)) => {
            // Seen edge-on the plane cannot be followed: the instances stay until the view allows it.
            let Some(at) = gizmo::ray_plane(ray, f.origin, f.dirs[normal], gizmo::MIN_PLANE_FACING * 0.6) else {
                return;
            };
            let rel = at - f.origin - drag.grab;
            let (a, b) = gizmo::plane_axes(normal);
            let (da, db) = (
                gizmo::snap(rel.dot(f.dirs[a]), step),
                gizmo::snap(rel.dot(f.dirs[b]), step),
            );
            drag.moved = [0.0; 3];
            drag.moved[a] = da;
            drag.moved[b] = db;
            f.dirs[a] * da + f.dirs[b] * db
        }
    };
    let starts = drag.starts.clone();
    scene.watching(&drag.watched, |dm| {
        for start in starts {
            let target = CFrame::from_parts(start.pivot.position() + delta, start.pivot.rotation());
            if let Err(error) = world_object::pivot_to(dm, start.id, target) {
                log::warn!("tools: could not move: {error}");
            }
        }
    });
}

/// The tools' part of a viewport frame: what is under the pointer, starting, running and ending a drag on the
/// panel's `response`. Returns whether the pointer is on the gizmo, so a click there does not select.
pub fn interact(
    ui: &egui::Ui,
    response: &egui::Response,
    view: &View,
    scene: &mut Scene<'_>,
    gizmo: &mut Gizmo,
) -> bool {
    if let Some(mut drag) = gizmo.drag.take() {
        if ui.input(|input| input.pointer.primary_down()) {
            if let Some(ray) = ui.input(|input| input.pointer.latest_pos()).and_then(|at| view.ray(at)) {
                update_drag(scene, &mut drag, ray, 0.0);
            }
            gizmo.drag = Some(drag);
        }
        return true;
    }
    let targets = targets(scene.dm, scene.selection);
    let frame = frame_for(scene.dm, gizmo.tool, gizmo.local, &targets, view.pos);
    gizmo.hover = frame
        .zip(response.hover_pos())
        .and_then(|(frame, at)| hit(gizmo.tool, &frame, view, at));
    let alt = ui.input(|input| input.modifiers.alt);
    if response.drag_started_by(egui::PointerButton::Primary) && !alt {
        let origin = ui.input(|input| input.pointer.press_origin());
        let grabbed = frame.zip(origin).and_then(|(frame, at)| {
            let handle = hit(gizmo.tool, &frame, view, at)?;
            begin_drag(scene.dm, handle, frame, view.ray(at)?, &targets)
        });
        gizmo.drag = grabbed;
    }
    gizmo.hover.is_some() || gizmo.drag.is_some()
}

/// The X, Y and Z colours: muted, easy to tell apart, quiet against the scene (Forge's).
const AXIS_RGB: [[u8; 3]; 3] = [[214, 96, 96], [118, 186, 118], [96, 140, 214]];
/// The colour of the handle under the pointer, and of a drag's measurement.
const HIGHLIGHT_RGB: [u8; 3] = [236, 214, 120];

fn colour(rgb: [u8; 3], alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha)
}

fn axis_colour(axis: usize, lit: bool, alpha: u8) -> Color32 {
    colour(if lit { HIGHLIGHT_RGB } else { AXIS_RGB[axis] }, alpha)
}

/// Draws the tool's gizmo for the selection, or while dragging the drag's measurement (the handles step aside).
pub fn paint(painter: &Painter, view: &View, scene: &Scene<'_>, gizmo: &Gizmo) {
    if let Some(drag) = &gizmo.drag {
        paint_measurement(painter, view, drag);
        return;
    }
    let targets = targets(scene.dm, scene.selection);
    let Some(frame) = frame_for(scene.dm, gizmo.tool, gizmo.local, &targets, view.pos) else {
        return;
    };
    if gizmo.tool == Tool::Move {
        paint_move(painter, view, &frame, gizmo.hover);
    }
}

/// The Move gizmo: plane squares, then arrows (a shaft from the gap to a solid cone).
fn paint_move(painter: &Painter, view: &View, frame: &Frame, hover: Option<Handle>) {
    let lit = |handle: MoveHandle| hover == Some(Handle::Move(handle));
    let signs = gizmo::facing_signs(view.pos, frame.origin, &frame.dirs);
    for axis in 0..3 {
        let corners = gizmo::plane_corners(frame.origin, &frame.dirs, &signs, axis, frame.len);
        if let Some(points) = corners.iter().map(|c| view.project(*c)).collect::<Option<Vec<Pos2>>>() {
            let on = lit(MoveHandle::Plane(axis));
            painter.add(Shape::convex_polygon(
                points,
                axis_colour(axis, on, if on { 190 } else { 110 }),
                Stroke::new(1.0, axis_colour(axis, on, 220)),
            ));
        }
    }
    for (axis, &facing) in signs.iter().enumerate() {
        for sign in [1.0f32, -1.0] {
            let on = lit(MoveHandle::Arrow(axis, sign));
            // The half pointing away from the camera is a little fainter.
            let alpha = if sign == facing { 255 } else { 170 };
            let dir = frame.dirs[axis] * sign;
            let start = frame.origin + dir * (frame.len * gizmo::ARROW_GAP);
            let base = frame.origin + dir * (frame.len * (1.0 - gizmo::CONE_LEN));
            let tip = frame.origin + dir * frame.len;
            if let Some(line) = view.segment(start, base) {
                painter.line_segment(line, Stroke::new(2.5, axis_colour(axis, on, alpha)));
            }
            cone(
                painter,
                view,
                base,
                tip,
                frame.len * gizmo::CONE_RADIUS,
                axis_colour(axis, on, alpha),
            );
        }
    }
}

/// A solid cone, flat: the outline of its base circle and tip on the screen.
fn cone(painter: &Painter, view: &View, base: Vec3, tip: Vec3, radius: f32, fill: Color32) {
    let points = gizmo::cone_points(base, tip, radius, 14);
    let Some(screen) = points
        .iter()
        .map(|p| view.project(*p).map(|s| Vec2::new(s.x, s.y)))
        .collect::<Option<Vec<Vec2>>>()
    else {
        return;
    };
    let hull = gizmo::convex_hull(&screen);
    if hull.len() >= 3 {
        painter.add(Shape::convex_polygon(
            hull.iter().map(|p| pos2(p.x, p.y)).collect(),
            fill,
            Stroke::NONE,
        ));
    }
}

/// How far apart a ruler's marks are, in world units.
const RULER_STEP: f32 = 5.0;

/// A drag's measurement: for a move, a ruler along each axis it moved on, marked every [`RULER_STEP`], with the
/// distance where the instances are now.
fn paint_measurement(painter: &Painter, view: &View, drag: &Drag) {
    let f = &drag.frame;
    let axes: Vec<usize> = match drag.handle {
        Handle::Move(MoveHandle::Arrow(axis, _)) => vec![axis],
        Handle::Move(MoveHandle::Plane(normal)) => {
            let (a, b) = gizmo::plane_axes(normal);
            vec![a, b]
        }
    };
    for axis in axes {
        ruler(
            painter,
            view,
            f.origin,
            f.dirs[axis],
            AXIS_RGB[axis],
            f.len,
            drag.moved[axis],
        );
        if let Some(at) = view.project(f.origin + f.dirs[axis] * drag.moved[axis]) {
            label(
                painter,
                at,
                &format!("{} {:+.2}", ["X", "Y", "Z"][axis], drag.moved[axis]),
                AXIS_RGB[axis],
            );
        }
    }
    if let Some(at) = view.project(f.origin) {
        painter.circle_filled(at, 3.5, colour(HIGHLIGHT_RGB, 255));
    }
}

/// A ruler from `origin` along `dir`: the stretch travelled (`moved`) bright, a faint guide past both ends, and marks
/// across it every [`RULER_STEP`], longest at the start and where it ends now. `size` is the gizmo's length.
fn ruler(painter: &Painter, view: &View, origin: Vec3, dir: Vec3, rgb: [u8; 3], size: f32, moved: f32) {
    let at = |s: f32| origin + dir * s;
    let (lo, hi) = (moved.min(0.0) - RULER_STEP, moved.max(0.0) + RULER_STEP);
    for (from, to, stroke) in [
        (lo, moved.min(0.0), Stroke::new(1.5, colour(rgb, 110))),
        (moved.max(0.0), hi, Stroke::new(1.5, colour(rgb, 110))),
        (0.0, moved, Stroke::new(2.5, colour(HIGHLIGHT_RGB, 255))),
    ] {
        if (to - from).abs() > 1e-4
            && let Some(line) = view.segment(at(from), at(to))
        {
            painter.line_segment(line, stroke);
        }
    }
    let across = dir.cross(view.pos - origin).normalize_or_zero();
    let tick = |s: f32, half: f32, color: Color32| {
        if let Some(line) = view.segment(at(s) - across * half, at(s) + across * half) {
            painter.line_segment(line, Stroke::new(1.5, color));
        }
    };
    let small = size * 0.06;
    let first = (lo / RULER_STEP).ceil() as i32;
    let last = (hi / RULER_STEP).floor() as i32;
    // Never more than a couple of hundred marks, however far the drag went.
    let stride = ((last - first).max(1) / 200).max(1);
    for k in (first..=last).filter(|k| k % stride == 0) {
        tick(k as f32 * RULER_STEP, small, colour(rgb, 230));
    }
    tick(0.0, small * 1.8, colour(HIGHLIGHT_RGB, 255));
    tick(moved, small * 2.4, colour(HIGHLIGHT_RGB, 255));
}

/// Text on a dark plate beside `at`, readable on any background.
fn label(painter: &Painter, at: Pos2, text: &str, rgb: [u8; 3]) {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::monospace(13.0), colour(rgb, 255));
    let rect = egui::Align2::LEFT_BOTTOM.anchor_size(at + egui::vec2(12.0, -10.0), galley.size());
    painter.rect_filled(
        rect.expand2(egui::vec2(5.0, 2.0)),
        4.0,
        Color32::from_rgba_unmultiplied(15, 15, 18, 215),
    );
    painter.galley(rect.min, galley, colour(rgb, 255));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::History;
    use crate::panels::explorer::ExplorerState;
    use crate::viewport::EditorCamera;
    use egui::Rect;
    use metallic_engine::metallic_core::Value;

    #[test]
    fn a_move_drag_along_an_arrow_moves_every_target_from_where_it_started_as_one_step() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = pick::workspace(&dm).unwrap();
        let part = |dm: &mut DataModel, at: Vec3| {
            let id = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
            dm.set_parent(id, Some(world)).unwrap();
            id
        };
        let a = part(&mut dm, Vec3::new(0.0, 0.0, 0.0));
        let b = part(&mut dm, Vec3::new(4.0, 0.0, 0.0));
        let mut selection = Selection::of([a, b]);
        let mut explorer = ExplorerState::new(&dm);
        let mut history = History::new(selection.clone());
        let mut clipboard = Default::default();
        let camera = EditorCamera::looking_at(Vec3::new(2.0, 10.0, 30.0), Vec3::new(2.0, 0.0, 0.0));
        let view = View::new(&camera, Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0)));

        let targets = targets(&dm, &selection);
        let frame = frame_for(&dm, Tool::Move, false, &targets, view.pos).unwrap();
        assert!(
            frame.origin.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-4),
            "the middle of both"
        );
        assert_eq!(frame.dirs, gizmo::AXES);
        assert_eq!(frame_for(&dm, Tool::Select, false, &targets, view.pos), None);
        // Take hold of the +X arrow and pull it 3 along X.
        let on_arrow = view.project(frame.origin + Vec3::X * frame.len * 0.6).unwrap();
        let handle = hit(Tool::Move, &frame, &view, on_arrow).unwrap();
        assert_eq!(handle, Handle::Move(MoveHandle::Arrow(0, 1.0)));
        let mut drag = begin_drag(&dm, handle, frame, view.ray(on_arrow).unwrap(), &targets).unwrap();
        let mut scene = Scene {
            dm: &mut dm,
            selection: &mut selection,
            explorer: &mut explorer,
            history: &mut history,
            clipboard: &mut clipboard,
            playing: false,
        };
        let pulled = view.project(frame.origin + Vec3::X * (frame.len * 0.6 + 3.4)).unwrap();
        for _ in 0..3 {
            // The same pointer, frame after frame: the same place, nothing added up.
            update_drag(&mut scene, &mut drag, view.ray(pulled).unwrap(), 0.0);
        }
        assert!((drag.moved[0] - 3.4).abs() < 1e-3, "{:?}", drag.moved);
        // With a step, it lands on whole steps.
        update_drag(&mut scene, &mut drag, view.ray(pulled).unwrap(), 2.0);
        assert!((drag.moved[0] - 4.0).abs() < 1e-3);
        let now = std::time::Instant::now();
        scene.commit(now, false);
        let position = |dm: &DataModel, id| world_object::pivot(dm, id).unwrap().position();
        assert!(position(scene.dm, a).abs_diff_eq(Vec3::new(4.0, 0.0, 0.0), 1e-3));
        assert!(position(scene.dm, b).abs_diff_eq(Vec3::new(8.0, 0.0, 0.0), 1e-3));
        // One undo step takes both back.
        assert!(scene.undo(now));
        assert!(position(scene.dm, a).abs_diff_eq(Vec3::ZERO, 1e-4));
        assert!(position(scene.dm, b).abs_diff_eq(Vec3::new(4.0, 0.0, 0.0), 1e-4));
    }
}
