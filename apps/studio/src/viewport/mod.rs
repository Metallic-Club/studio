//! The viewport (ADR 0009, Viewport): the engine's render of the open game in the Viewport panel, seen from the
//! editor camera, which is the game's `Workspace.CurrentCamera` (#576).
//!
//! - **The texture.** The panel notes its size in physical pixels ([`ViewportState::pixels`]) and shows
//!   [`ViewportState::texture`] filling itself. The shell (`app::viewport`) owns the engine side: after each frame's
//!   interface it sizes the engine's viewport target to the panel, draws the game into it from the editor camera, and
//!   registers the texture with egui-wgpu (again when a resize replaced it), before the frame is painted. While a
//!   resize is under way the old texture is stretched over the new size, so there is never a black frame.
//! - **The camera** ([`EditorCamera`]) moves through the keymap's viewport commands ([`ViewportState::navigate`],
//!   after Forge-Engine's controls, #385): keys need the viewport's focus, the mouse and wheel start over it. While a
//!   look or an orbit goes on, the shell holds the pointer where the drag began ([`ViewportState::hold`]). As in
//!   Roblox Studio it is the game's `Workspace.CurrentCamera` (#576, [`ViewportState::sync`]): each move is written
//!   into the camera's `CFrame` and `FieldOfView`, as no undo step and no unsaved change, and editing those (the
//!   Properties panel, undo) moves the view. Not while a game runs: its camera is its own.
//! - **Frame pacing.** While the panel is shown and the window has focus it asks for frames continuously (at the
//!   Rendering settings' frame cap, if any); otherwise Studio draws only on input or change.
//! - **No game scripts run.** The editor shows the scene as it is; Play mode is later work.

pub mod axes;
pub mod camera;
pub mod gizmo;
pub mod light_icons;
pub mod pick;
pub mod select;
pub mod shapes;
pub mod tools;
pub mod view;

use std::time::Instant;

use egui::{Pos2, Rect, TextureId};
use metallic_engine::metallic_core::glam::{Vec2, Vec3};
use metallic_engine::metallic_core::{DataModel, Id, Value};

pub use camera::EditorCamera;
pub use view::View;

use metallic_engine::perf::{StatsViewer, StatsViewers};

use crate::keymap::{Activation, ActivationKind, Keymap, ids};
use crate::settings::{FLY_SPEED, SECONDARY_SPEED};

/// How far ahead the camera takes there to be something when nothing is in front (open space), in studs.
const OPEN_REACH: f32 = 20.0;

/// Which camera speed the wheel last changed, for the viewport's readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedChange {
    /// The fly speed.
    Fly,
    /// The secondary speed.
    Secondary,
}

/// What a frame gives the camera besides the keymap.
#[derive(Clone, Copy, Debug)]
pub struct NavInput<'a> {
    /// The mouse's movement this frame: its own (raw) movement while the pointer is held, else the pointer's.
    pub motion: egui::Vec2,
    /// Where the pointer is, if anywhere.
    pub pointer: Option<Pos2>,
    /// The frame's length, in seconds.
    pub dt: f32,
    /// Shift is held: fly at the secondary speed.
    pub fast: bool,
    /// The scene, to aim at.
    pub dm: &'a DataModel,
    /// What is selected, the active instance last: what Focus frames.
    pub selection: &'a [Id],
}

/// The viewport's state: its camera, and what the panel and the shell tell each other.
#[derive(Clone, Debug)]
pub struct ViewportState {
    /// The editor camera.
    pub camera: EditorCamera,
    /// What keeps the editor camera and the game's current camera one view (#576).
    pub sync: camera::CameraSync,
    /// The panel's area as last drawn, in points.
    pub rect: Option<Rect>,
    /// The panel's size as last drawn, in physical pixels: the size the engine draws at.
    pub pixels: Option<(u32, u32)>,
    /// Whether the panel was drawn since the shell last drew the scene: a panel behind another tab, or closed, is not
    /// drawn into.
    pub shown: bool,
    /// The engine's render, registered with the UI renderer, once the shell has drawn one.
    pub texture: Option<TextureId>,
    /// The size of the target that render is in, as last drawn: kept larger than the panel, whose frame fills its
    /// top-left corner (#330).
    pub texture_pixels: Option<(u32, u32)>,
    /// The largest side that target can grow to (the device's largest texture).
    pub texture_limit: u32,
    /// Why the engine cannot draw the viewport, when it cannot (no renderer could be made).
    pub error: Option<String>,
    /// The transform tool and its gizmo (#355).
    pub gizmo: tools::Gizmo,
    /// Where the pointer is held, in points, while a look or an orbit goes on (#385).
    pub hold: Option<Pos2>,
    /// The speed the wheel last changed, and when: the viewport shows it for a moment.
    pub speed_note: Option<(SpeedChange, Instant)>,
    /// When the Select tool's mode last changed (#467): the viewport shows it for a moment.
    pub mode_note: Option<Instant>,
    /// The walk of Alt+clicks through the parts behind one another (#482).
    pub pick_cycle: pick::PickCycle,
    /// The viewport's device draws lines, so the wireframe shading view can be chosen (#478). False until the viewport
    /// draws.
    pub wireframe: bool,
    /// The performance overlay's viewers shown over the viewport (#479): its Debug options and Shift+F1 to F4.
    pub stats: StatsViewers,
}

impl ViewportState {
    /// The viewport of `dm`, its camera framing the game's.
    pub fn new(dm: &DataModel) -> Self {
        Self {
            camera: EditorCamera::framing(dm),
            sync: camera::CameraSync::default(),
            rect: None,
            pixels: None,
            shown: false,
            texture: None,
            texture_pixels: None,
            texture_limit: u32::MAX,
            error: None,
            gizmo: tools::Gizmo::default(),
            hold: None,
            speed_note: None,
            mode_note: None,
            pick_cycle: pick::PickCycle::default(),
            wireframe: false,
            stats: StatsViewers::NONE,
        }
    }

    /// Moves the camera by this frame's viewport commands (#385): `activations` the keymap fired and the commands it
    /// holds, with `input`. The wheel's speed commands change `speeds`. Returns whether the camera moved.
    pub fn navigate(
        &mut self,
        keymap: &Keymap,
        activations: &[Activation],
        input: NavInput<'_>,
        speeds: &mut crate::settings::Viewport,
    ) -> bool {
        let before = self.camera;
        let active = input.selection.last().copied();
        self.camera.keep_focus_on(active);
        let view = self.rect.map(|rect| View::new(&self.camera, rect));
        for activation in activations {
            match (activation.command, activation.kind) {
                (ids::LOOK, ActivationKind::Press) => {
                    self.camera.begin_look(None);
                    self.hold = input.pointer;
                }
                (ids::ORBIT, ActivationKind::Press) => {
                    // Round what is under the pointer: the first part its ray meets, else the ground, else a point
                    // ahead.
                    let ray = input
                        .pointer
                        .zip(view)
                        .and_then(|(at, view)| view.ray(at))
                        .unwrap_or((self.camera.position, self.camera.forward()));
                    let (reach, _) = reach_along(input.dm, ray);
                    self.camera
                        .begin_look(Some(ray.0 + ray.1 * reach.max(camera::ORBIT_MIN_DISTANCE)));
                    self.hold = input.pointer;
                }
                (ids::STATS_FRAME_RATE, ActivationKind::Press) => self.stats.toggle(StatsViewer::FrameRate),
                (ids::STATS_DRAWS, ActivationKind::Press) => self.stats.toggle(StatsViewer::Draws),
                (ids::STATS_LIGHTING, ActivationKind::Press) => self.stats.toggle(StatsViewer::Lighting),
                (ids::STATS_GRAPHICS, ActivationKind::Press) => self.stats.toggle(StatsViewer::Graphics),
                (ids::FOCUS, ActivationKind::Press) => {
                    if let Some((centre, radius)) = bounds_of(input.dm, input.selection) {
                        let aspect = self.rect.map_or(1.5, |rect| rect.aspect_ratio());
                        self.camera.focus_on(centre, radius, aspect, active);
                    }
                }
                (ids::ZOOM, ActivationKind::Wheel(lines)) => {
                    let ahead = reach_along(input.dm, (self.camera.position, self.camera.forward()));
                    self.camera.zoom(lines, ahead);
                }
                (ids::SPEED, ActivationKind::Wheel(lines)) => {
                    speeds.fly_speed = (speeds.fly_speed * 1.1f32.powf(lines)).clamp(FLY_SPEED.0, FLY_SPEED.1);
                    self.speed_note = Some((SpeedChange::Fly, Instant::now()));
                }
                (ids::SECONDARY_SPEED, ActivationKind::Wheel(lines)) => {
                    speeds.secondary_speed =
                        (speeds.secondary_speed * 1.1f32.powf(lines)).clamp(SECONDARY_SPEED.0, SECONDARY_SPEED.1);
                    self.speed_note = Some((SpeedChange::Secondary, Instant::now()));
                }
                _ => {}
            }
        }
        if keymap.is_held(ids::LOOK) || keymap.is_held(ids::ORBIT) {
            self.camera.turn(Vec2::new(input.motion.x, input.motion.y));
        } else {
            self.camera.end_look();
            self.hold = None;
        }
        let axis = |plus: &str, minus: &str| f32::from(keymap.is_held(plus)) - f32::from(keymap.is_held(minus));
        let forward = axis(ids::FLY_FORWARD, ids::FLY_BACK);
        let right = axis(ids::FLY_RIGHT, ids::FLY_LEFT);
        let up = axis(ids::FLY_UP, ids::FLY_DOWN);
        if forward != 0.0 || right != 0.0 || up != 0.0 {
            let speed = speeds.fly_speed * if input.fast { speeds.secondary_speed } else { 1.0 };
            self.camera.fly(forward, right, up, speed, input.dt);
        }
        self.camera.step(input.dt);
        self.camera != before
    }
}

/// How far along `ray` what is in front is: the first part it meets, or the ground (`true`, a surface not to pass);
/// else (`false`) a distance that suits how high it starts, so zooming and orbiting still go somewhere in open space.
fn reach_along(dm: &DataModel, ray: view::Ray) -> (f32, bool) {
    if let Some((_, t)) = pick::nearest_part(dm, ray) {
        return (t, true);
    }
    let (origin, dir) = ray;
    if dir.y < -1e-3 {
        let t = -origin.y / dir.y;
        if t > 0.0 && t < 2000.0 {
            return (t, true);
        }
    }
    (OPEN_REACH.max(origin.y.abs() * 0.5), false)
}

/// The middle of the box round `ids`' outlines and the radius of the sphere round it: what Focus frames.
fn bounds_of(dm: &DataModel, ids: &[Id]) -> Option<(Vec3, f32)> {
    let corners: Vec<Vec3> = ids
        .iter()
        .filter_map(|&id| pick::outline_box(dm, id))
        .flat_map(|bounds| bounds.corners())
        .collect();
    let min = corners.iter().copied().reduce(Vec3::min);
    let max = corners.iter().copied().reduce(Vec3::max);
    match min.zip(max) {
        Some((min, max)) => Some(((min + max) * 0.5, ((max - min) * 0.5).length().max(0.5))),
        // Nothing with a box: where they are.
        None => {
            let places: Vec<Vec3> = ids.iter().filter_map(|&id| position_of(dm, id)).collect();
            (!places.is_empty()).then(|| (places.iter().sum::<Vec3>() / places.len() as f32, 2.0))
        }
    }
}

/// Where the editor camera looks (#412): a point, and the outward normal of the surface it is on, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    /// The point.
    pub point: Vec3,
    /// The surface's outward normal there; `None` in open space.
    pub normal: Option<Vec3>,
}

/// Where `camera` looks (#412): the first part's surface the middle of its view meets, else where that line meets the
/// ground (y = 0, facing up), else a point [`OPEN_REACH`] ahead.
pub fn aim(dm: &DataModel, camera: &EditorCamera) -> Aim {
    let ray = (camera.position, camera.forward());
    if let Some((t, normal)) = pick::nearest_face(dm, ray) {
        return Aim {
            point: ray.0 + ray.1 * t,
            normal: Some(normal),
        };
    }
    match reach_along(dm, ray) {
        (t, true) => Aim {
            point: ray.0 + ray.1 * t,
            normal: Some(Vec3::Y),
        },
        (t, false) => Aim {
            point: ray.0 + ray.1 * t,
            normal: None,
        },
    }
}

/// Where instance `id` is, if it is placed in the world (a part, a model with a pivot): what an orbit circles.
pub fn position_of(dm: &DataModel, id: Id) -> Option<Vec3> {
    match dm.get_by_name(id, "CFrame") {
        Ok(Value::CFrame(cframe)) => Some(cframe.position()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, PointerButton, pos2};

    use super::*;
    use crate::keymap::{Context, Focus, Overrides};

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn button(button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos: pos2(10.0, 10.0),
            button,
            pressed,
            modifiers,
        }
    }

    #[test]
    fn the_keymaps_commands_drive_the_camera() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let workspace = pick::workspace(&dm).unwrap();
        let part = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        let at = metallic_engine::metallic_core::CFrame::new(Vec3::new(30.0, 2.0, 0.0));
        dm.set_by_name(part, "CFrame", Value::CFrame(at)).unwrap();
        dm.set_parent(part, Some(workspace)).unwrap();
        let mut viewport = ViewportState::new(&dm);
        viewport.rect = Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let mut speeds = crate::settings::Viewport::default();
        let mut keymap = Keymap::studio(&Overrides::new());
        let focused = Focus {
            hovered: Some(Context::Viewport),
            ..Focus::new(Context::Viewport)
        };
        let start = viewport.camera;
        let input = |motion: egui::Vec2, fast: bool, selection: &'static [Id]| NavInput {
            motion,
            pointer: Some(egui::pos2(400.0, 300.0)),
            dt: 0.05,
            fast,
            dm: &dm,
            selection,
        };
        let mut nav = |viewport: &mut ViewportState, fired: &[Activation], keymap: &Keymap, input: NavInput<'_>| {
            viewport.navigate(keymap, fired, input, &mut speeds)
        };

        // W held flies forward at the fly speed; with Shift it still flies, at the secondary speed.
        let fired = keymap
            .process(&[key(Key::W, true, Modifiers::NONE)], focused)
            .activations;
        assert!(nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, &[])));
        let moved = (viewport.camera.position - start.position).length();
        assert!((moved - 2.0).abs() < 1e-3, "40 studs a second for 0.05 s: {moved}");
        keymap.process(&[key(Key::W, false, Modifiers::NONE)], focused);
        assert!(
            !nav(&mut viewport, &[], &keymap, input(egui::Vec2::ZERO, false, &[])),
            "released"
        );
        let fired = keymap
            .process(&[key(Key::W, true, Modifiers::SHIFT)], focused)
            .activations;
        let before = viewport.camera.position;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, true, &[]));
        assert!(
            (viewport.camera.position.distance(before) - 6.0).abs() < 1e-3,
            "three times as fast"
        );
        keymap.process(&[key(Key::W, false, Modifiers::SHIFT)], focused);

        // A right drag looks, holding the pointer where it began.
        let fired = keymap
            .process(&[button(PointerButton::Secondary, true, Modifiers::NONE)], focused)
            .activations;
        let yaw = viewport.camera.yaw();
        assert!(nav(
            &mut viewport,
            &fired,
            &keymap,
            input(egui::vec2(30.0, 0.0), false, &[])
        ));
        assert!(viewport.camera.yaw() < yaw);
        assert_eq!(viewport.hold, Some(egui::pos2(400.0, 300.0)));
        keymap.process(&[button(PointerButton::Secondary, false, Modifiers::NONE)], focused);
        nav(&mut viewport, &[], &keymap, input(egui::Vec2::ZERO, false, &[]));
        assert_eq!(viewport.hold, None, "let go when the drag ends");

        // Alt and a right drag orbit the point under the pointer: the ground ahead, which keeps its distance.
        let fired = keymap
            .process(&[button(PointerButton::Secondary, true, Modifiers::ALT)], focused)
            .activations;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, &[]));
        let view = View::new(&viewport.camera, viewport.rect.unwrap());
        let ray = view.ray(egui::pos2(400.0, 300.0)).unwrap();
        let ground = ray.0 + ray.1 * (-ray.0.y / ray.1.y);
        let distance = viewport.camera.position.distance(ground);
        nav(&mut viewport, &[], &keymap, input(egui::vec2(50.0, 10.0), false, &[]));
        assert!((viewport.camera.position.distance(ground) - distance).abs() < 1e-2);
        keymap.process(&[button(PointerButton::Secondary, false, Modifiers::ALT)], focused);

        // F frames the selection.
        let selected: &'static [Id] = Box::leak(Box::new([part]));
        let fired = keymap
            .process(&[key(Key::F, true, Modifiers::NONE)], focused)
            .activations;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, selected));
        for _ in 0..10 {
            nav(&mut viewport, &[], &keymap, input(egui::Vec2::ZERO, false, selected));
        }
        let focus = viewport.camera.focus().expect("focused");
        assert!(focus.pivot.distance(Vec3::new(30.0, 2.0, 0.0)) < 1e-3);
        assert!(viewport.camera.position.distance(focus.pivot) < 20.0);

        // The wheel zooms; with Alt it changes the fly speed, with Shift the secondary one.
        let wheel = |modifiers| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        let at = viewport.camera.position;
        let fired = keymap.process(&[wheel(Modifiers::NONE)], focused).activations;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, selected));
        assert!(viewport.camera.position != at);
        let fired = keymap.process(&[wheel(Modifiers::ALT)], focused).activations;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, selected));
        let fired = keymap.process(&[wheel(Modifiers::SHIFT)], focused).activations;
        nav(&mut viewport, &fired, &keymap, input(egui::Vec2::ZERO, false, selected));
        assert!((speeds.fly_speed - 44.0).abs() < 1e-3 && (speeds.secondary_speed - 3.3).abs() < 1e-3);
        assert_eq!(viewport.speed_note.map(|note| note.0), Some(SpeedChange::Secondary));
    }

    #[test]
    fn a_parts_position_is_its_cframes() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let part = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        let at = metallic_engine::metallic_core::CFrame::new(Vec3::new(1.0, 2.0, 3.0));
        dm.set_by_name(part, "CFrame", Value::CFrame(at)).unwrap();
        assert_eq!(position_of(&dm, part), Some(Vec3::new(1.0, 2.0, 3.0)));
        let folder = dm.create(dm.registry().class_id("Folder").unwrap()).unwrap();
        assert_eq!(position_of(&dm, folder), None);
    }
}
