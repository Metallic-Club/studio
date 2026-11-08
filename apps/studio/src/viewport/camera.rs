//! The editor camera (#385, after Forge-Engine's): Studio's own point of view on the game, never the game's
//! `Workspace.CurrentCamera`, which it only reads once, to start where the game's camera is.
//!
//! - **Looking** (a right drag) turns it where it stands. **Orbiting** (Alt with a drag) swings it round a pivot, the
//!   point that was under the pointer when the drag began, which stays where it is on the screen. While either goes
//!   on, Studio holds the pointer where it was (still shown) and turns by the mouse's own movement, so a drag never
//!   runs into the edge of the screen ([`super::ViewportState::hold`]).
//! - **Focus** glides to frame the selection; from then on, until the camera is flown by hand or the selection
//!   changes, a right drag orbits what it framed and the wheel moves toward it.
//! - **Flying** goes at the fly speed, or at the secondary speed (a multiple of it, below one for slow, careful moves)
//!   with Shift held. Both are settings, which the wheel changes.
//! - **The wheel** zooms toward what is ahead: each notch a tenth of the way to the surface in front, never through it,
//!   or a fixed share of a guess in open space, travelled over a few frames.

use metallic_engine::metallic_core::glam::{Quat, Vec2, Vec3};
use metallic_engine::metallic_core::{DataModel, Id};
use metallic_engine::metallic_render::{Camera, CameraReader};

/// How far the camera turns per unit the mouse moves while looking or orbiting, in radians (Forge's).
const TURN_PER_UNIT: f32 = 0.003;
/// The steepest the camera looks up or down: just short of straight, where yaw would flip.
const MAX_PITCH: f32 = 88.0 * std::f32::consts::PI / 180.0;
/// The longest frame a fly step covers, in seconds: a stall does not throw the camera across the world.
const MAX_STEP: f32 = 0.1;
/// A wheel notch covers this share of the way to what is ahead...
const ZOOM_STEP: f32 = 0.10;
/// ...but never less than this many studs a notch...
const ZOOM_MIN_STEP: f32 = 0.2;
/// ...nor more.
const ZOOM_MAX_STEP: f32 = 150.0;
/// Zooming in stops this far short of a surface.
const ZOOM_STOP: f32 = 0.5;
/// How quickly a zoom is travelled: the share left after a second is `e^-ZOOM_RATE` (most of it in a third of one).
const ZOOM_RATE: f32 = 14.0;
/// How long the glide to a focus takes, in seconds.
const GLIDE_TIME: f32 = 0.22;
/// The nearest an orbit's pivot may be.
pub const ORBIT_MIN_DISTANCE: f32 = 0.5;

/// What a held drag is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Idle,
    /// Turning in place.
    Free,
    /// Swinging round a pivot for the length of the drag.
    Pivot(Vec3),
    /// Swinging round the focus.
    Focused,
}

/// The orbit Focus sets: what the camera circles and how far away, and what it framed (the active instance), which
/// leaving ends it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// The middle of what was framed.
    pub pivot: Vec3,
    /// How far the camera is from it.
    pub distance: f32,
    /// The active instance it framed.
    pub target: Option<Id>,
}

/// The editor camera: where it is, which way it looks (yaw and pitch, no roll), and what it is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCamera {
    /// World position, in studs.
    pub position: Vec3,
    /// Turn about the world's up axis, in radians; 0 looks along −Z.
    yaw: f32,
    /// Tilt up (positive) or down, in radians.
    pitch: f32,
    /// The vertical field of view, in radians.
    fov_y: f32,
    look: Look,
    focus: Option<Focus>,
    /// A glide to a focus: from, to, and how far along (0 to 1).
    glide: Option<(Vec3, Vec3, f32)>,
    /// The zoom asked for and not yet travelled (positive: closer), and its way.
    zoom: (f32, Vec3),
}

impl EditorCamera {
    /// A camera at `position` looking at `target`, with a 70° field of view.
    pub fn looking_at(position: Vec3, target: Vec3) -> Self {
        let mut camera = Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            look: Look::Idle,
            focus: None,
            glide: None,
            zoom: (0.0, Vec3::NEG_Z),
        };
        camera.aim(target - position);
        camera
    }

    /// A camera where `camera` is, looking the same way.
    pub fn from_camera(camera: &Camera) -> Self {
        let mut this = Self::looking_at(camera.position, camera.position + camera.rotation * Vec3::NEG_Z);
        this.fov_y = camera.fov_y;
        this
    }

    /// The camera a project opens with: where the game's `Workspace.CurrentCamera` is, or looking at the origin from
    /// above and behind when the game has none.
    pub fn framing(dm: &DataModel) -> Self {
        let workspace = dm.registry().class_id("Workspace").and_then(|class| dm.service(class));
        match workspace.and_then(|world| CameraReader::new().read(dm, world)) {
            Some(camera) => Self::from_camera(&camera),
            None => Self::looking_at(Vec3::new(0.0, 20.0, 40.0), Vec3::ZERO),
        }
    }

    /// Turns the camera to look along `direction` (ignored when it is zero).
    fn aim(&mut self, direction: Vec3) {
        let Some(forward) = direction.try_normalize() else {
            return;
        };
        self.yaw = (-forward.x).atan2(-forward.z);
        self.pitch = forward.y.clamp(-1.0, 1.0).asin().clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Its rotation: −Z forward, +Y up.
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    /// The way it looks.
    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    /// Its right.
    pub fn right(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    /// Turn about the up axis, in radians.
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Tilt, in radians.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// The vertical field of view, in radians.
    pub fn fov_y(&self) -> f32 {
        self.fov_y
    }

    /// The orbit Focus set, while it lasts.
    pub fn focus(&self) -> Option<Focus> {
        self.focus
    }

    /// Whether a drag is turning it.
    pub fn is_looking(&self) -> bool {
        self.look != Look::Idle
    }

    /// What the renderer draws from.
    pub fn camera(&self) -> Camera {
        Camera {
            position: self.position,
            rotation: self.rotation(),
            fov_y: self.fov_y,
            near: Camera::DEFAULT_NEAR,
        }
    }

    /// Starts a drag: an orbit round `pivot`, or else (none given) an orbit round the focus when there is one, or a
    /// look.
    pub fn begin_look(&mut self, pivot: Option<Vec3>) {
        self.look = match pivot {
            Some(pivot) => {
                // The pivot is the drag's alone: the focus and a zoom under way are left behind.
                self.focus = None;
                self.zoom.0 = 0.0;
                Look::Pivot(pivot)
            }
            None if self.focus.is_some() => Look::Focused,
            None => Look::Free,
        };
    }

    /// Ends a drag.
    pub fn end_look(&mut self) {
        self.look = Look::Idle;
    }

    /// Turns by the mouse's movement `delta` during a drag: right turns right, down looks down. An orbit swings the
    /// whole camera round its pivot by the same turn, so the pivot stays where it is on the screen.
    pub fn turn(&mut self, delta: Vec2) {
        if self.look == Look::Idle {
            return;
        }
        let before = self.rotation();
        self.yaw -= delta.x * TURN_PER_UNIT;
        self.pitch = (self.pitch - delta.y * TURN_PER_UNIT).clamp(-MAX_PITCH, MAX_PITCH);
        match self.look {
            Look::Pivot(pivot) => {
                let turn = self.rotation() * before.inverse();
                self.position = pivot + turn * (self.position - pivot);
            }
            Look::Focused => {
                if let Some(focus) = self.focus {
                    self.place_around(focus.pivot, focus.distance);
                }
            }
            Look::Idle | Look::Free => {}
        }
    }

    /// Puts the camera `distance` from `pivot`, looking at it the way it looks now.
    fn place_around(&mut self, pivot: Vec3, distance: f32) {
        self.position = pivot - self.forward() * distance;
    }

    /// Flies for `dt` seconds at `speed`: `forward` along the view, `right` sideways, `up` along the world's up. Each is
    /// −1, 0 or 1; diagonals are no faster. Not during an orbit or a glide; flying by hand ends the focus.
    pub fn fly(&mut self, forward: f32, right: f32, up: f32, speed: f32, dt: f32) {
        if matches!(self.look, Look::Pivot(_) | Look::Focused) || self.glide.is_some() {
            return;
        }
        let direction = self.forward() * forward + self.right() * right + Vec3::Y * up;
        if let Some(direction) = direction.try_normalize() {
            self.position += direction * speed * dt.clamp(0.0, MAX_STEP);
            self.focus = None;
        }
    }

    /// Glides to frame a sphere round `centre`, `radius` big, in a view `aspect` wide for each unit high, keeping the
    /// way it looks; from then on it orbits it, the focus of `target`.
    pub fn focus_on(&mut self, centre: Vec3, radius: f32, aspect: f32, target: Option<Id>) {
        let distance = frame_distance(radius, self.fov_y, aspect);
        self.zoom.0 = 0.0;
        self.glide = Some((self.position, centre - self.forward() * distance, 0.0));
        self.focus = Some(Focus {
            pivot: centre,
            distance,
            target,
        });
    }

    /// Ends the focus unless it framed `active` (the selection's active instance): something else is selected.
    pub fn keep_focus_on(&mut self, active: Option<Id>) {
        if self.focus.is_some_and(|focus| focus.target != active) {
            self.focus = None;
        }
    }

    /// Asks for `notches` of zoom (positive: in). `ahead` is how far what is in front is, and whether it is a surface
    /// not to pass; with a focus, the zoom goes toward it instead.
    pub fn zoom(&mut self, notches: f32, ahead: (f32, bool)) {
        let (reach, blocked) = match self.focus {
            Some(focus) => (focus.distance, true),
            None => ahead,
        };
        let notches = notches.clamp(-6.0, 6.0);
        // Toward a surface each notch closes a tenth of what is left, so n in and n out end where they began; in open
        // space every notch is as long.
        let wanted = if blocked {
            reach * (1.0 - (1.0 - ZOOM_STEP).powf(notches))
        } else {
            reach * ZOOM_STEP * notches
        };
        let n = notches.abs();
        let mut travel = wanted.signum() * wanted.abs().clamp(ZOOM_MIN_STEP * n, ZOOM_MAX_STEP * n);
        if blocked && travel > 0.0 {
            // Closer, but not through it, counting what is on its way already.
            travel = travel.min((reach - ZOOM_STOP - self.zoom.0.max(0.0)).max(0.0));
        }
        let way = self.forward();
        if self.zoom.0 != 0.0 && self.zoom.1.dot(way) < 0.9 {
            // The view turned since: the old way is forgotten.
            self.zoom.0 = 0.0;
        }
        self.zoom = (self.zoom.0 + travel, way);
    }

    /// Goes on with a glide or a zoom for `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, MAX_STEP);
        if let Some((from, to, t)) = self.glide {
            let t = (t + dt / GLIDE_TIME).min(1.0);
            let eased = t * t * (3.0 - 2.0 * t);
            self.position = from.lerp(to, eased);
            self.glide = (t < 1.0).then_some((from, to, t));
        }
        let (left, way) = self.zoom;
        if left != 0.0 {
            let step = if left.abs() < 5e-4 {
                left
            } else {
                left * (1.0 - (-ZOOM_RATE * dt).exp())
            };
            self.zoom.0 -= step;
            match self.focus.as_mut() {
                Some(focus) => {
                    focus.distance = (focus.distance - step).clamp(ORBIT_MIN_DISTANCE, 5000.0);
                    let (pivot, distance) = (focus.pivot, focus.distance);
                    self.place_around(pivot, distance);
                }
                None => self.position += way * step,
            }
        }
    }
}

/// How far from a sphere of `radius` the camera must be for it to fill a view of `fov_y`, `aspect` wide for each unit
/// high: the narrower of the two angles decides.
pub fn frame_distance(radius: f32, fov_y: f32, aspect: f32) -> f32 {
    let half_v = fov_y * 0.5;
    let half_h = (half_v.tan() * aspect.max(0.1)).atan();
    (radius / half_v.min(half_h).sin() * 1.15).clamp(3.0, 5000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn it_starts_where_the_games_camera_is() {
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let game = CameraReader::new().read(&dm, workspace).unwrap();
        let camera = EditorCamera::framing(&dm);
        assert!(close(camera.position, game.position));
        assert!(close(camera.forward(), game.rotation * Vec3::NEG_Z));
        assert!((camera.camera().fov_y - game.fov_y).abs() < 1e-6);
    }

    #[test]
    fn looking_at_a_point_faces_it() {
        let camera = EditorCamera::looking_at(Vec3::new(0.0, 20.0, 40.0), Vec3::ZERO);
        assert!(close(camera.forward(), Vec3::new(0.0, -20.0, -40.0).normalize()));
        assert!(close(camera.camera().rotation * Vec3::NEG_Z, camera.forward()));
        assert!(close(camera.right(), Vec3::X));
    }

    #[test]
    fn flying_moves_along_the_view_at_the_speed_given() {
        let mut camera = EditorCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
        camera.fly(1.0, 0.0, 0.0, 20.0, 0.05);
        assert!(close(camera.position, Vec3::new(0.0, 0.0, -1.0)));
        camera.fly(0.0, 1.0, 1.0, 20.0, 0.05);
        let moved = camera.position - Vec3::new(0.0, 0.0, -1.0);
        assert!((moved.length() - 1.0).abs() < 1e-3, "diagonals are no faster");
        // A long stall moves only as far as a tenth of a second.
        let before = camera.position;
        camera.fly(-1.0, 0.0, 0.0, 20.0, 5.0);
        assert!((camera.position.distance(before) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn looking_turns_in_place_and_stops_short_of_straight_up() {
        let mut camera = EditorCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
        camera.turn(Vec2::new(40.0, 0.0));
        assert!(camera.forward().z < -0.999, "no drag, no turn");
        camera.begin_look(None);
        camera.turn(Vec2::new(40.0, 0.0));
        assert!(camera.forward().x > 0.1, "moving right turns right");
        camera.turn(Vec2::new(0.0, -100_000.0));
        assert!(camera.pitch() <= MAX_PITCH && camera.forward().y > 0.99);
        assert_eq!(camera.position, Vec3::ZERO, "looking does not move it");
    }

    #[test]
    fn an_orbit_swings_round_its_pivot_which_stays_put_on_the_screen() {
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 5.0, 20.0), Vec3::ZERO);
        // A pivot off to one side of the view: it stays in the same direction from the camera, in its own axes.
        let pivot = Vec3::new(4.0, 0.0, 0.0);
        let seen = |camera: &EditorCamera| camera.rotation().inverse() * (pivot - camera.position);
        let before = seen(&camera);
        camera.begin_look(Some(pivot));
        camera.turn(Vec2::new(120.0, 30.0));
        assert!(
            (seen(&camera) - before).length() < 1e-3,
            "{:?} vs {before:?}",
            seen(&camera)
        );
        camera.end_look();
        assert!(!camera.is_looking());
    }

    #[test]
    fn focus_glides_to_frame_and_then_right_drags_and_the_wheel_go_round_and_toward_it() {
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO);
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let target = dm.registry().class_id("Workspace").and_then(|class| dm.service(class));
        camera.focus_on(Vec3::new(2.0, 0.0, 0.0), 1.0, 1.5, target);
        for _ in 0..20 {
            camera.step(1.0 / 30.0);
        }
        let focus = camera.focus().unwrap();
        assert!(
            close(camera.position, focus.pivot - camera.forward() * focus.distance),
            "arrived"
        );
        assert!(focus.distance < 10.0);
        // A right drag orbits it at its distance.
        camera.begin_look(None);
        camera.turn(Vec2::new(200.0, 50.0));
        assert!((camera.position.distance(focus.pivot) - focus.distance).abs() < 1e-3);
        assert!(close(camera.forward(), (focus.pivot - camera.position).normalize()));
        camera.end_look();
        // The wheel goes toward it, never past it.
        for _ in 0..50 {
            camera.zoom(3.0, (0.0, false));
            camera.step(0.05);
        }
        let near = camera.position.distance(focus.pivot);
        assert!(near >= ORBIT_MIN_DISTANCE - 1e-3 && near < focus.distance, "{near}");
        // Something else selected ends it; so does flying.
        camera.keep_focus_on(target);
        assert!(camera.focus().is_some());
        camera.keep_focus_on(None);
        assert!(camera.focus().is_none());
        camera.focus_on(Vec3::ZERO, 1.0, 1.0, None);
        camera.step(1.0);
        camera.step(1.0);
        camera.step(1.0);
        camera.fly(1.0, 0.0, 0.0, 10.0, 0.05);
        assert!(camera.focus().is_none(), "flying by hand leaves it");
    }

    #[test]
    fn the_wheel_zooms_toward_a_surface_without_passing_it() {
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
        let ahead = |camera: &EditorCamera| (camera.position.z, true);
        camera.zoom(1.0, ahead(&camera));
        for _ in 0..30 {
            camera.step(0.05);
        }
        assert!(
            (camera.position.z - 9.0).abs() < 1e-2,
            "a tenth of the way: {}",
            camera.position.z
        );
        for _ in 0..100 {
            camera.zoom(6.0, ahead(&camera));
            camera.step(0.05);
        }
        assert!(
            camera.position.z >= ZOOM_STOP - 1e-3,
            "never through it: {}",
            camera.position.z
        );
        // In open space every notch is as long.
        let mut open = EditorCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
        open.zoom(2.0, (20.0, false));
        for _ in 0..30 {
            open.step(0.05);
        }
        assert!((open.position.z + 4.0).abs() < 1e-2, "{}", open.position.z);
    }

    #[test]
    fn framing_fits_the_narrower_angle() {
        let tall = frame_distance(2.0, 70f32.to_radians(), 0.5);
        let wide = frame_distance(2.0, 70f32.to_radians(), 2.0);
        assert!(tall > wide, "a narrow view stands further back");
    }
}
