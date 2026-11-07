//! The editor camera: Studio's own point of view on the game, flown and orbited by the keymap's viewport commands.
//! It is never the game's `Workspace.CurrentCamera`, which it only reads once, to start where the game's camera is.

use metallic_engine::metallic_core::DataModel;
use metallic_engine::metallic_core::glam::{Quat, Vec2, Vec3};
use metallic_engine::metallic_render::{Camera, CameraReader};

/// How far the camera turns per point the pointer moves while looking or orbiting, in radians (a quarter degree).
const TURN_PER_POINT: f32 = 0.25 * std::f32::consts::PI / 180.0;
/// The steepest the camera looks up or down: just short of straight, where yaw would flip.
const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;
/// The closest the camera zooms to what it looks at, in metres.
const MIN_FOCUS: f32 = 0.5;
/// The fly speeds the wheel goes between, in metres per second.
const SPEEDS: (f32, f32) = (1.0, 1000.0);
/// The longest frame a fly step covers, in seconds: a stall does not throw the camera across the world.
const MAX_STEP: f32 = 0.1;

/// The editor camera: where it is, which way it looks (yaw and pitch, no roll), and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCamera {
    /// World position, in metres.
    pub position: Vec3,
    /// Turn about the world's up axis, in radians; 0 looks along −Z.
    yaw: f32,
    /// Tilt up (positive) or down, in radians.
    pitch: f32,
    /// The vertical field of view, in radians.
    fov_y: f32,
    /// How fast it flies, in metres per second.
    speed: f32,
    /// How far ahead what it looks at is: what it orbits without a selection, and what zoom closes in on.
    focus: f32,
    /// The point it orbits while an orbit is held.
    pivot: Option<Vec3>,
}

impl EditorCamera {
    /// The speed a new camera flies at, in metres per second.
    pub const DEFAULT_SPEED: f32 = 20.0;

    /// A camera at `position` looking at `target`, with a 70° field of view.
    pub fn looking_at(position: Vec3, target: Vec3) -> Self {
        let mut camera = Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            speed: Self::DEFAULT_SPEED,
            focus: position.distance(target).max(MIN_FOCUS),
            pivot: None,
        };
        camera.aim(target - position);
        camera
    }

    /// A camera where `camera` is, looking the same way, its focus where its view meets the ground.
    pub fn from_camera(camera: &Camera) -> Self {
        let mut this = Self::looking_at(camera.position, camera.position + camera.rotation * Vec3::NEG_Z);
        this.fov_y = camera.fov_y;
        let forward = this.forward();
        // Looking down: what it looks at is where the view meets the ground (y = 0).
        this.focus = if forward.y < -0.05 {
            (-camera.position.y / forward.y).clamp(1.0, 500.0)
        } else {
            20.0
        };
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

    /// How fast it flies, in metres per second.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// How far ahead what it looks at is, in metres.
    pub fn focus(&self) -> f32 {
        self.focus
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

    /// Flies for `dt` seconds at its speed: `forward` along the view, `right` sideways, `up` along the world's up.
    /// Each is −1, 0 or 1; diagonals are no faster.
    pub fn fly(&mut self, forward: f32, right: f32, up: f32, dt: f32) {
        let direction = self.forward() * forward + self.right() * right + Vec3::Y * up;
        if let Some(direction) = direction.try_normalize() {
            self.position += direction * self.speed * dt.clamp(0.0, MAX_STEP);
        }
    }

    /// Turns where it stands by the pointer's movement `delta`, in points: right turns right, down looks down.
    pub fn look(&mut self, delta: Vec2) {
        self.yaw -= delta.x * TURN_PER_POINT;
        self.pitch = (self.pitch - delta.y * TURN_PER_POINT).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Starts orbiting `pivot` (the selection's position), or what it looks at when there is none: it turns to face
    /// the pivot, keeping its distance.
    pub fn begin_orbit(&mut self, pivot: Option<Vec3>) {
        let pivot = pivot.unwrap_or(self.position + self.forward() * self.focus);
        self.focus = self.position.distance(pivot).max(MIN_FOCUS);
        self.aim(pivot - self.position);
        self.pivot = Some(pivot);
    }

    /// Circles the pivot by the pointer's movement `delta`, in points, keeping its distance. Without an orbit
    /// begun, it begins one around what it looks at.
    pub fn orbit(&mut self, delta: Vec2) {
        if self.pivot.is_none() {
            self.begin_orbit(None);
        }
        let Some(pivot) = self.pivot else {
            return;
        };
        self.look(delta);
        self.position = pivot - self.forward() * self.focus;
    }

    /// Ends an orbit.
    pub fn end_orbit(&mut self) {
        self.pivot = None;
    }

    /// Moves in (positive `lines`, the wheel away from the user) or out, by a share of the distance to what it
    /// looks at, so a wheel turn feels the same near and far. It never passes what it looks at.
    pub fn zoom(&mut self, lines: f32) {
        let step = lines * (self.focus * 0.12).max(0.25);
        let step = step.min(self.focus - MIN_FOCUS);
        self.position += self.forward() * step;
        self.focus = (self.focus - step).max(MIN_FOCUS);
        if let Some(pivot) = self.pivot {
            self.focus = self.position.distance(pivot).max(MIN_FOCUS);
        }
    }

    /// Flies faster (positive `lines`) or slower, a quarter per wheel line.
    pub fn change_speed(&mut self, lines: f32) {
        self.speed = (self.speed * 1.25f32.powf(lines)).clamp(SPEEDS.0, SPEEDS.1);
    }
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
        assert!((camera.focus() - Vec3::new(0.0, 20.0, 40.0).length()).abs() < 1e-3);
        // The rotation the renderer gets agrees with the forward vector.
        assert!(close(camera.camera().rotation * Vec3::NEG_Z, camera.forward()));
        assert!(close(camera.right(), Vec3::X));
    }

    #[test]
    fn flying_moves_along_the_view_at_the_speed() {
        let mut camera = EditorCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
        camera.fly(1.0, 0.0, 0.0, 0.05);
        assert!(close(camera.position, Vec3::new(0.0, 0.0, -1.0)));
        camera.fly(0.0, 1.0, 1.0, 0.05);
        let moved = camera.position - Vec3::new(0.0, 0.0, -1.0);
        assert!((moved.length() - 1.0).abs() < 1e-3, "diagonals are no faster");
        assert!(moved.x > 0.0 && moved.y > 0.0);
        // A long stall moves only as far as a tenth of a second.
        let before = camera.position;
        camera.fly(-1.0, 0.0, 0.0, 5.0);
        assert!((camera.position.distance(before) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn looking_turns_and_stops_short_of_straight_up() {
        let mut camera = EditorCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
        camera.look(Vec2::new(40.0, 0.0));
        assert!(camera.forward().x > 0.1, "moving right turns right");
        camera.look(Vec2::new(0.0, -100_000.0));
        assert!(camera.pitch() <= MAX_PITCH && camera.forward().y > 0.99);
        assert_eq!(camera.position, Vec3::ZERO, "looking does not move it");
    }

    #[test]
    fn orbiting_circles_the_pivot_at_its_distance() {
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
        let pivot = Vec3::new(5.0, 0.0, 0.0);
        camera.begin_orbit(Some(pivot));
        let distance = camera.position.distance(pivot);
        assert!(
            close(camera.forward(), (pivot - camera.position).normalize()),
            "it faces the pivot"
        );
        camera.orbit(Vec2::new(120.0, 30.0));
        assert!((camera.position.distance(pivot) - distance).abs() < 1e-3);
        assert!(close(camera.forward(), (pivot - camera.position).normalize()));
        camera.end_orbit();

        // Without a selection, around what it looks at.
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
        camera.orbit(Vec2::new(200.0, 0.0));
        assert!((camera.position.length() - 10.0).abs() < 1e-3);
        assert!(close(camera.forward(), -camera.position.normalize()));
    }

    #[test]
    fn zoom_closes_in_without_passing_and_speed_scales() {
        let mut camera = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
        camera.zoom(1.0);
        assert!(camera.position.z < 10.0 && camera.position.z > 0.0);
        for _ in 0..200 {
            camera.zoom(3.0);
        }
        assert!(camera.position.z >= MIN_FOCUS - 1e-3, "{:?}", camera.position);
        camera.zoom(-2.0);
        assert!(camera.focus() > MIN_FOCUS);

        let speed = camera.speed();
        camera.change_speed(1.0);
        assert!((camera.speed() - speed * 1.25).abs() < 1e-3);
        camera.change_speed(-100.0);
        assert_eq!(camera.speed(), SPEEDS.0);
    }
}
