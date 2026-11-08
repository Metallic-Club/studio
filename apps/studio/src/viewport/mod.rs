//! The viewport (ADR 0009, Viewport): the engine's render of the open game in the Viewport panel, seen from the
//! editor's own camera.
//!
//! - **The texture.** The panel notes its size in physical pixels ([`ViewportState::pixels`]) and shows
//!   [`ViewportState::texture`] filling itself. The shell (`app::viewport`) owns the engine side: after each frame's
//!   interface it sizes the engine's viewport target to the panel, draws the game into it from the editor camera, and
//!   registers the texture with egui-wgpu (again when a resize replaced it), before the frame is painted. While a
//!   resize is under way the old texture is stretched over the new size, so there is never a black frame.
//! - **The camera** ([`EditorCamera`]) is Studio's, not the game's `Workspace.CurrentCamera`: the game is shown, never
//!   changed. It moves only through the keymap's viewport commands ([`ViewportState::navigate`]): keys need the
//!   viewport's focus, the mouse and wheel start over it.
//! - **Frame pacing.** While the panel is shown and the window has focus it asks for frames continuously (at the
//!   Rendering settings' frame cap, if any); otherwise Studio draws only on input or change.
//! - **No game scripts run.** The editor shows the scene as it is; Play mode is later work.

pub mod camera;
pub mod gizmo;
pub mod pick;
pub mod tools;
pub mod view;

use egui::{Rect, TextureId};
use metallic_engine::metallic_core::glam::{Vec2, Vec3};
use metallic_engine::metallic_core::{DataModel, Id, Value};

pub use camera::EditorCamera;
pub use view::View;

use crate::keymap::{Activation, ActivationKind, Keymap, ids};

/// The viewport's state: its camera, and what the panel and the shell tell each other.
#[derive(Clone, Debug)]
pub struct ViewportState {
    /// The editor camera.
    pub camera: EditorCamera,
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
}

impl ViewportState {
    /// The viewport of `dm`, its camera framing the game's.
    pub fn new(dm: &DataModel) -> Self {
        Self {
            camera: EditorCamera::framing(dm),
            rect: None,
            pixels: None,
            shown: false,
            texture: None,
            texture_pixels: None,
            texture_limit: u32::MAX,
            error: None,
            gizmo: tools::Gizmo::default(),
        }
    }

    /// Moves the camera by this frame's viewport commands: `activations` the keymap fired, the commands it holds,
    /// the pointer's movement `pointer` (points) and the frame's `dt` (seconds). `pivot` is what an orbit circles:
    /// the selection's position, if any. Returns whether the camera moved.
    pub fn navigate(
        &mut self,
        keymap: &Keymap,
        activations: &[Activation],
        pointer: egui::Vec2,
        dt: f32,
        pivot: Option<Vec3>,
    ) -> bool {
        let before = self.camera;
        for activation in activations {
            match (activation.command, activation.kind) {
                (ids::ORBIT, ActivationKind::Press) => self.camera.begin_orbit(pivot),
                (ids::ZOOM, ActivationKind::Wheel(lines)) => self.camera.zoom(lines),
                (ids::SPEED, ActivationKind::Wheel(lines)) => self.camera.change_speed(lines),
                _ => {}
            }
        }
        let delta = Vec2::new(pointer.x, pointer.y);
        if keymap.is_held(ids::ORBIT) {
            self.camera.orbit(delta);
        } else {
            self.camera.end_orbit();
            if keymap.is_held(ids::LOOK) {
                self.camera.look(delta);
            }
        }
        let axis = |plus: &str, minus: &str| f32::from(keymap.is_held(plus)) - f32::from(keymap.is_held(minus));
        let forward = axis(ids::FLY_FORWARD, ids::FLY_BACK);
        let right = axis(ids::FLY_RIGHT, ids::FLY_LEFT);
        let up = axis(ids::FLY_UP, ids::FLY_DOWN);
        if forward != 0.0 || right != 0.0 || up != 0.0 {
            self.camera.fly(forward, right, up, dt);
        }
        self.camera != before
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

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
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
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let mut viewport = ViewportState::new(&dm);
        let mut keymap = Keymap::studio(&Overrides::new());
        let focused = Focus {
            hovered: Some(Context::Viewport),
            ..Focus::new(Context::Viewport)
        };
        let start = viewport.camera;

        // W held flies forward.
        let fired = keymap.process(&[key(Key::W, true)], focused).activations;
        assert!(viewport.navigate(&keymap, &fired, egui::Vec2::ZERO, 0.05, None));
        assert!((viewport.camera.position - start.position).dot(start.forward()) > 0.0);
        keymap.process(&[key(Key::W, false)], focused);
        assert!(
            !viewport.navigate(&keymap, &[], egui::Vec2::ZERO, 0.05, None),
            "released"
        );

        // Right-drag looks.
        let fired = keymap
            .process(&[button(PointerButton::Secondary, true, Modifiers::NONE)], focused)
            .activations;
        assert!(viewport.navigate(&keymap, &fired, egui::vec2(30.0, 0.0), 0.016, None));
        assert!(viewport.camera.yaw() < start.yaw());
        keymap.process(&[button(PointerButton::Secondary, false, Modifiers::NONE)], focused);

        // Alt+left-drag orbits the selection.
        let pivot = Vec3::new(3.0, 0.0, 0.0);
        let fired = keymap
            .process(&[button(PointerButton::Primary, true, Modifiers::ALT)], focused)
            .activations;
        viewport.navigate(&keymap, &fired, egui::vec2(50.0, 0.0), 0.016, Some(pivot));
        let distance = viewport.camera.position.distance(pivot);
        viewport.navigate(&keymap, &[], egui::vec2(50.0, 10.0), 0.016, Some(pivot));
        assert!((viewport.camera.position.distance(pivot) - distance).abs() < 1e-3);
        keymap.process(&[button(PointerButton::Primary, false, Modifiers::ALT)], focused);

        // The wheel zooms; with Shift it changes the speed.
        let wheel = |modifiers| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        let at = viewport.camera.position;
        let fired = keymap.process(&[wheel(Modifiers::NONE)], focused).activations;
        viewport.navigate(&keymap, &fired, egui::Vec2::ZERO, 0.016, None);
        assert!(viewport.camera.position != at);
        let fired = keymap.process(&[wheel(Modifiers::SHIFT)], focused).activations;
        viewport.navigate(&keymap, &fired, egui::Vec2::ZERO, 0.016, None);
        assert!(viewport.camera.speed() > EditorCamera::DEFAULT_SPEED);
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
