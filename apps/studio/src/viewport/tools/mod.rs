//! The transform tools in the viewport (#355), after Forge-Engine's `tools.rs`: **Select, Move, Scale, Rotate**, in
//! world or local axes. The tool's gizmo sits on the selection: on the active instance's pivot for one, at the middle
//! of everything selected for several. [`super::gizmo`] has the maths; each tool's handles, drag and drawing are in a
//! module of their own ([`moving`], [`scale`], [`rotate`]; [`carry`] picks parts up), and this module is what they share: which tool, what it acts on, the
//! pointer, and the measurements' line and labels.
//!
//! **Drags are measured from where they began**: each frame puts what the drag can change back as it was when the drag
//! started, then places the instances from there and from the pointer, never from the frame before, so nothing adds
//! up rounding, and a key pressed half way only changes the formula. Putting it back matters for a model without a
//! `PrimaryPart`: it keeps no rotation, so the engine measures its pivot from its parts, and moving it to a pivot
//! from the start while it is still turned from the last frame would turn it again (#377). A drag is one undo step: the button is held throughout (see `editor::history`), and the
//! engine code a drag runs writes the data model, so the scene notes the properties it changed
//! ([`Scene::watching`]).

mod carry;
mod moving;
mod rotate;
mod scale;

pub use rotate::quarter_turn;

use egui::{Color32, Painter, Pos2, Stroke};
use metallic_engine::metallic_classes::base::world_object;
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, DataModel, Id, PropId, Value};

use super::gizmo::{self, MoveHandle};
use super::pick::{self, Bounds};
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

/// A handle of the active tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// One of the Move gizmo's.
    Move(MoveHandle),
    /// A Scale sphere: its axis (0..3) and side (+1 or -1).
    Scale(usize, f32),
    /// A Rotate ring, by the axis it turns about.
    Ring(usize),
    /// The trackball inside the rings.
    Trackball,
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
    /// The Scale tool's box: one part's own, or the world-aligned box round several.
    pub bounds: Option<Bounds>,
}

/// The increments drags snap to (#359); 0 is free.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snap {
    /// Moves and sizes, in studs.
    pub move_step: f32,
    /// Turns, in degrees.
    pub turn_step: f32,
}

/// The keys a drag reads as it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// Shift: scale all three sizes by one factor.
    pub shift: bool,
    /// Ctrl (Cmd): scale both sides about the centre.
    pub ctrl: bool,
}

/// An instance a drag moves, as it was when the drag began.
#[derive(Clone, Copy, Debug)]
struct Start {
    id: Id,
    pivot: CFrame,
}

/// What a drag of each kind keeps.
#[derive(Clone, Debug)]
enum Kind {
    Move(moving::MoveDrag),
    Scale(scale::ScaleDrag),
    Rotate(rotate::RotateDrag),
    Carry(carry::CarryDrag),
}

/// A drag in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    frame: Frame,
    starts: Vec<Start>,
    /// Every property the drag can change, which the scene notes for undo.
    watched: Vec<(Id, PropId)>,
    /// Those properties' values when the drag began, in the same order: each frame starts from them.
    rest: Vec<Option<Value>>,
    kind: Kind,
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

/// The world-aligned box round `targets`' outlines.
fn span(dm: &DataModel, targets: &[Id]) -> Option<Bounds> {
    let corners: Vec<Vec3> = targets
        .iter()
        .filter_map(|&id| pick::outline_box(dm, id))
        .flat_map(|bounds| bounds.corners())
        .collect();
    let min = corners.iter().copied().reduce(Vec3::min)?;
    let max = corners.iter().copied().reduce(Vec3::max)?;
    Some(Bounds {
        center: (min + max) * 0.5,
        half: ((max - min) * 0.5).max(Vec3::splat(0.001)),
        rot: Default::default(),
    })
}

/// Where `tool`'s handles are for `targets` (the active one last), seen from `camera`. `None` for the Select tool and
/// for nothing to act on.
pub fn frame_for(dm: &DataModel, tool: Tool, local: bool, targets: &[Id], camera: Vec3) -> Option<Frame> {
    let &active = targets.last()?;
    if tool == Tool::Select {
        return None;
    }
    let pivot = world_object::pivot(dm, active).ok()?;
    let (origin, dirs, bounds) = match (tool, targets) {
        // One part or model: its own box and axes (a size is along its own edges; a model's turn with it, #413).
        (Tool::Scale, [only]) if pick::outline_box(dm, *only).is_some() => {
            let own = pick::outline_box(dm, *only)?;
            (own.center, gizmo::axis_dirs(true, own.rot), Some(own))
        }
        (Tool::Scale, _) => {
            let around = span(dm, targets)?;
            (around.center, gizmo::AXES, Some(around))
        }
        (_, [_]) => (pivot.position(), gizmo::axis_dirs(local, pivot.rotation()), None),
        (_, _) => (
            span(dm, targets)?.center,
            gizmo::axis_dirs(local, pivot.rotation()),
            None,
        ),
    };
    Some(Frame {
        origin,
        dirs,
        len: gizmo::handle_len(camera, origin),
        bounds,
    })
}

/// The handle of `tool` under `pointer`.
pub fn hit(tool: Tool, frame: &Frame, view: &View, pointer: Pos2) -> Option<Handle> {
    match tool {
        Tool::Move => gizmo::hit_move(view, pointer, frame.origin, &frame.dirs).map(Handle::Move),
        Tool::Scale => {
            let ray = view.ray(pointer)?;
            gizmo::hit_scale(view.pos, ray, frame.bounds.as_ref()?).map(|(axis, sign)| Handle::Scale(axis, sign))
        }
        Tool::Rotate => rotate::hit(frame, view.pos, view.ray(pointer)?),
        Tool::Select => None,
    }
}

/// What moving or turning instances changes: their `CFrame`s, and a model's kept pivot (#413).
pub(super) const MOVED: &[&str] = &["CFrame", "WorldPivotData", "WorldPivotSet"];

/// `name` of every instance in and under `targets` that has it: what a drag can change.
fn watch_list(dm: &DataModel, targets: &[Id], names: &[&str]) -> Vec<(Id, PropId)> {
    let mut out = Vec::new();
    for &target in targets {
        let inside = dm.descendants(target).into_iter().flatten();
        for id in std::iter::once(target).chain(inside) {
            out.extend(
                names
                    .iter()
                    .filter_map(|name| dm.prop_id(id, name).ok().map(|prop| (id, prop))),
            );
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
    let (kind, watched) = match handle {
        Handle::Move(handle) => (
            Kind::Move(moving::begin(handle, &frame, ray)?),
            watch_list(dm, targets, MOVED),
        ),
        Handle::Scale(axis, sign) => (
            Kind::Scale(scale::begin(dm, axis, sign, &frame, ray, targets)?),
            watch_list(dm, targets, &["CFrame", "Size", "WorldPivotData", "WorldPivotSet"]),
        ),
        Handle::Ring(axis) => (
            Kind::Rotate(rotate::begin(Some(axis), &frame, ray)?),
            watch_list(dm, targets, MOVED),
        ),
        Handle::Trackball => (
            Kind::Rotate(rotate::begin(None, &frame, ray)?),
            watch_list(dm, targets, MOVED),
        ),
    };
    Some(Drag {
        frame,
        starts,
        rest: values(dm, &watched),
        watched,
        kind,
    })
}

/// The values of `watched` now.
fn values(dm: &DataModel, watched: &[(Id, PropId)]) -> Vec<Option<Value>> {
    watched.iter().map(|&(id, prop)| dm.get(id, prop).ok()).collect()
}

/// Picks up the selection with `ray` (#358), when the part it meets is selected or inside something selected.
pub fn carry(dm: &DataModel, selection: &Selection, ray: Ray) -> Option<Drag> {
    let (part, t) = pick::nearest_part(dm, ray)?;
    if !selection.contains(pick::selectable(dm, part)) {
        return None;
    }
    let targets = targets(dm, selection);
    let grab = ray.0 + ray.1 * t;
    let kind = Kind::Carry(carry::begin(dm, &targets, grab, ray)?);
    let watched = watch_list(dm, &targets, MOVED);
    Some(Drag {
        frame: Frame {
            origin: grab,
            dirs: gizmo::AXES,
            len: 1.0,
            bounds: None,
        },
        starts: targets
            .iter()
            .filter_map(|&id| world_object::pivot(dm, id).ok().map(|pivot| Start { id, pivot }))
            .collect(),
        rest: values(dm, &watched),
        watched,
        kind,
    })
}

/// Follows the pointer's `ray` with the `keys` held: places the instances from where they started, as part of the
/// scene's action under way, snapped to `snap`.
pub fn update_drag(scene: &mut Scene<'_>, drag: &mut Drag, ray: Ray, keys: Keys, snap: Snap) {
    let watched = std::mem::take(&mut drag.watched);
    scene.watching(&watched, |dm| {
        for (&(id, prop), value) in watched.iter().zip(&drag.rest) {
            if let Some(value) = value
                && let Err(error) = dm.set(id, prop, value.clone())
            {
                log::warn!("tools: could not put a drag back where it began: {error}");
            }
        }
        match &mut drag.kind {
            Kind::Move(state) => moving::update(dm, state, &drag.frame, &drag.starts, ray, snap.move_step),
            Kind::Scale(state) => scale::update(dm, state, &drag.frame, ray, keys, snap.move_step),
            Kind::Rotate(state) => rotate::update(dm, state, &drag.frame, &drag.starts, ray, snap.turn_step),
            Kind::Carry(state) => carry::update(dm, state, &drag.starts, ray, snap.move_step),
        }
    });
    drag.watched = watched;
}

/// The tools' part of a viewport frame: what is under the pointer, starting, running and ending a drag on the
/// panel's `response`. Returns whether the pointer is on the gizmo, so a click there does not select.
pub fn interact(
    ui: &egui::Ui,
    response: &egui::Response,
    view: &View,
    scene: &mut Scene<'_>,
    gizmo: &mut Gizmo,
    snap: Snap,
) -> bool {
    if let Some(mut drag) = gizmo.drag.take() {
        if ui.input(|input| input.pointer.primary_down()) {
            let (at, modifiers) = ui.input(|input| (input.pointer.latest_pos(), input.modifiers));
            let keys = Keys {
                shift: modifiers.shift,
                ctrl: modifiers.command,
            };
            if let Some(ray) = at.and_then(|at| view.ray(at)) {
                update_drag(scene, &mut drag, ray, keys, snap);
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
        let ray = origin.and_then(|at| view.ray(at));
        let handle = frame
            .zip(origin)
            .and_then(|(frame, at)| Some((frame, hit(gizmo.tool, &frame, view, at)?)));
        gizmo.drag = match (handle, ray) {
            (Some((frame, handle)), Some(ray)) => begin_drag(scene.dm, handle, frame, ray, &targets),
            // Off the handles, a drag on a part carries it (#358), selected first if it was not.
            (None, Some(ray)) => {
                if let Some(picked) = pick::pick(scene.dm, ray).filter(|&id| !scene.selection.contains(id)) {
                    scene.selection.set_only(Some(picked));
                }
                carry(scene.dm, scene.selection, ray)
            }
            _ => None,
        };
    }
    gizmo.hover.is_some() || gizmo.drag.is_some()
}

/// Draws the tool's gizmo for the selection, or while dragging the drag's measurement (the handles step aside),
/// a ring's marked every `snap.turn_step`.
pub fn paint(painter: &Painter, view: &View, scene: &Scene<'_>, gizmo: &Gizmo, snap: Snap) {
    if let Some(drag) = &gizmo.drag {
        match &drag.kind {
            Kind::Move(state) => moving::paint_measurement(painter, view, &drag.frame, state),
            Kind::Scale(state) => scale::paint_measurement(painter, view, &drag.frame, state),
            Kind::Rotate(state) => rotate::paint_measurement(painter, view, &drag.frame, state, snap.turn_step),
            // What is carried shows itself.
            Kind::Carry(_) => {}
        }
        return;
    }
    let targets = targets(scene.dm, scene.selection);
    let Some(frame) = frame_for(scene.dm, gizmo.tool, gizmo.local, &targets, view.pos) else {
        return;
    };
    match gizmo.tool {
        Tool::Move => moving::paint(painter, view, &frame, gizmo.hover),
        Tool::Scale => scale::paint(painter, view, &frame, gizmo.hover, targets.len() > 1),
        Tool::Rotate => rotate::paint(painter, view, &frame, gizmo.hover),
        Tool::Select => {}
    }
}

/// The X, Y and Z colours: muted, easy to tell apart, quiet against the scene (Forge's).
const AXIS_RGB: [[u8; 3]; 3] = [[214, 96, 96], [118, 186, 118], [96, 140, 214]];
/// The colour of the handle under the pointer, and of a drag's measurement.
const HIGHLIGHT_RGB: [u8; 3] = [236, 214, 120];
/// The axes' names, for the measurements.
pub(crate) const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

fn colour(rgb: [u8; 3], alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha)
}

pub(crate) fn axis_colour(axis: usize, lit: bool, alpha: u8) -> Color32 {
    colour(if lit { HIGHLIGHT_RGB } else { AXIS_RGB[axis] }, alpha)
}

/// A drag's trail (#376): a dot where it started, `from`, and a line from there to where it is now, `to`.
fn trail(painter: &Painter, view: &View, from: Vec3, to: Vec3) {
    let ink = colour(HIGHLIGHT_RGB, 255);
    if let Some(line) = view.segment(from, to) {
        painter.line_segment(line, Stroke::new(2.0, ink));
    }
    if let Some(at) = view.project(from) {
        painter.circle_filled(at, 4.0, ink);
    }
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
mod testing {
    //! A scene to drag in, for the tools' tests.

    use egui::Rect;
    use metallic_engine::metallic_core::Value;

    use super::*;
    use crate::editor::{Clipboard, History};
    use crate::panels::explorer::ExplorerState;
    use crate::viewport::EditorCamera;

    /// A game, what is selected in it, and what a [`Scene`] borrows.
    pub(super) struct Bench {
        pub(super) dm: DataModel,
        pub(super) selection: Selection,
        explorer: ExplorerState,
        history: History,
        clipboard: Clipboard,
    }

    impl Bench {
        pub(super) fn new() -> Self {
            let dm = metallic_engine::metallic_classes::new_game().unwrap();
            Self {
                explorer: ExplorerState::new(&dm),
                dm,
                selection: Selection::new(),
                history: History::default(),
                clipboard: Clipboard::default(),
            }
        }

        /// A part in Workspace at `at`, `size` big.
        pub(super) fn part(&mut self, at: Vec3, size: Vec3) -> Id {
            let dm = &mut self.dm;
            let id = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
            dm.set_by_name(id, "Size", Value::Vector3(size)).unwrap();
            dm.set_parent(id, pick::workspace(dm)).unwrap();
            id
        }

        /// Selects `ids`, as a step of the history already taken.
        pub(super) fn select(&mut self, ids: &[Id]) {
            self.selection = Selection::of(ids.iter().copied());
            self.history = History::new(self.selection.clone());
        }

        pub(super) fn scene(&mut self) -> Scene<'_> {
            Scene {
                dm: &mut self.dm,
                selection: &mut self.selection,
                explorer: &mut self.explorer,
                history: &mut self.history,
                clipboard: &mut self.clipboard,
                playing: false,
            }
        }
    }

    /// A view from `camera` looking at `target`, 1000 by 700 points.
    pub(super) fn look(camera: Vec3, target: Vec3) -> View {
        let camera = EditorCamera::looking_at(camera, target);
        View::new(&camera, Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0)))
    }

    /// The pivot's position of `id`.
    pub(super) fn position(dm: &DataModel, id: Id) -> Vec3 {
        world_object::pivot(dm, id).unwrap().position()
    }
}
