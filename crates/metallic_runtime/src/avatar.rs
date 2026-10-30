//! The default avatar: the skinned character drawn where the local character stands.
//!
//! The model is `art/avatar/out/default_avatar.glb`, built into the engine and imported once per
//! process. Each frame [`Avatar::update`] places it on the `HumanoidRootPart` (feet
//! [`ROOT_HEIGHT`] studs below the root centre, so they rest on the floor), turns it smoothly
//! toward the character's facing and animates it with the built-in
//! [`crate::locomotion::Locomotion`] state machine. The character's invisible body parts still do
//! the collisions and touches.
use crate::locomotion::{Locomotion, LocomotionInput};
use metallic_physics::ROOT_HEIGHT;
use metallic_render::glam::{Mat4, Vec3};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use metallic_render::{SkinStyle, SkinnedInstance};
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, OnceLock};

/// Length of one stud in metres (the avatar is authored in metres; 1.54 m = 5.5 studs).
pub(crate) const STUD_METRES: f32 = 0.28;
/// Time constant of the turn toward the move direction, in seconds: after this long the avatar
/// has turned 63% of the way, after three times as long 95%.
const TURN_TIME: f32 = 0.06;

const DEFAULT_AVATAR: &[u8] = include_bytes!("../../../art/avatar/out/default_avatar.glb");

/// The default avatar model, imported on first use. `None` (with a warning) if the built-in file
/// does not import, which the tests rule out.
pub(crate) fn default_model() -> Option<Arc<SkinnedModel>> {
    static MODEL: OnceLock<Option<Arc<SkinnedModel>>> = OnceLock::new();
    MODEL
        .get_or_init(|| match SkinnedModel::from_glb(DEFAULT_AVATAR) {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                eprintln!("warning: the default avatar could not be loaded ({e}); characters are invisible");
                None
            }
        })
        .clone()
}

/// The skinned avatar of one character.
pub(crate) struct Avatar {
    model: Arc<SkinnedModel>,
    locomotion: Locomotion,
    pose: Pose,
    /// Facing shown, radians around +Y (0 looks along -Z, like `Orientation.Y`).
    yaw: f32,
    /// Feet position in studs.
    feet: Vec3,
    joints: Vec<Mat4>,
}

impl Avatar {
    /// An avatar standing idle with its root centre at `root`, facing `yaw` radians.
    pub fn new(model: Arc<SkinnedModel>, root: Vec3, yaw: f32) -> Avatar {
        let locomotion = Locomotion::new(&model);
        let pose = Pose::rest(&model.skeleton);
        let mut avatar = Avatar {
            model,
            locomotion,
            pose,
            yaw,
            feet: root,
            joints: Vec::new(),
        };
        avatar.place(root);
        avatar
    }

    /// The animation controller (for tests and diagnostics).
    #[cfg(test)]
    pub fn locomotion(&self) -> &Locomotion {
        &self.locomotion
    }

    /// Facing shown, in radians.
    #[cfg(test)]
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Where the feet are, in studs.
    #[cfg(test)]
    pub fn feet(&self) -> Vec3 {
        self.feet
    }

    /// Advances by `dt` seconds: the character's root centre is at `root` and faces `target_yaw`
    /// radians; `input` drives the animation (its `turn_rate` is set here from the shown turn).
    pub fn update(&mut self, root: Vec3, target_yaw: f32, mut input: LocomotionInput, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        input.turn_rate = 0.0;
        if target_yaw.is_finite() {
            let diff = (target_yaw - self.yaw + PI).rem_euclid(TAU) - PI;
            let step = diff * (1.0 - (-dt / TURN_TIME).exp());
            self.yaw = (self.yaw + step).rem_euclid(TAU);
            if dt > 0.0 {
                input.turn_rate = step / dt;
            }
        }
        self.locomotion
            .update(&self.model, input, dt, &mut self.pose);
        self.place(root);
    }

    /// Recomputes the joint matrices for a root centre at `root`.
    fn place(&mut self, root: Vec3) {
        self.feet = root - Vec3::new(0.0, ROOT_HEIGHT as f32, 0.0);
        // The model faces +Z; Orientation.Y = 0 faces -Z, hence the half turn.
        let model = Mat4::from_translation(self.feet)
            * Mat4::from_rotation_y(self.yaw + PI)
            * Mat4::from_scale(Vec3::splat(1.0 / STUD_METRES));
        self.pose
            .skin_matrices(&self.model.skeleton, model, &mut self.joints);
    }

    /// What the renderer draws for this avatar.
    pub fn instance(&self) -> SkinnedInstance<'_> {
        let height = self.model.height() / STUD_METRES;
        SkinnedInstance {
            model: &self.model,
            joints: &self.joints,
            slot_colors: &[],
            center: self.feet + Vec3::new(0.0, height * 0.5, 0.0),
            height,
            detail_scale: 1.0 / STUD_METRES,
            style: SkinStyle::Toon,
        }
    }
}
