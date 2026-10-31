//! The default avatar: the skinned character drawn where the local character stands.
//!
//! The model is `art/avatar/out/default_avatar.glb`, built into the engine and imported once per
//! process ([`default_model`]). Each frame [`Avatar::update`] places it on the `HumanoidRootPart`
//! (feet [`ROOT_HEIGHT`] studs below the root centre, so they rest on the floor), turns it
//! smoothly toward the character's facing and animates it with a [`BodyAnimator`]: the same
//! CPU animation the simulation runs every tick for the touch volumes, so what is drawn is what
//! touches.
use crate::body_pose::{BodyAnimator, model_matrix};
pub(crate) use crate::body_pose::{STUD_METRES, default_model};
use crate::locomotion::LocomotionInput;
use metallic_physics::ROOT_HEIGHT;
use metallic_render::glam::{Mat4, Vec3};
use metallic_render::metallic_asset::SkinnedModel;
use metallic_render::{SkinStyle, SkinnedInstance};
use std::sync::Arc;

/// The skinned avatar of one character.
pub(crate) struct Avatar {
    animator: BodyAnimator,
    /// Feet position in studs.
    feet: Vec3,
    joints: Vec<Mat4>,
    /// The model matrix of the last [`Avatar::place`].
    model_matrix: Mat4,
}

impl Avatar {
    /// An avatar standing idle with its root centre at `root`, facing `yaw` radians.
    pub fn new(model: Arc<SkinnedModel>, root: Vec3, yaw: f32) -> Avatar {
        let mut avatar = Avatar {
            animator: BodyAnimator::new(model, yaw),
            feet: root,
            joints: Vec::new(),
            model_matrix: Mat4::IDENTITY,
        };
        avatar.place(root);
        avatar
    }

    /// The animation controller (for tests and diagnostics).
    #[cfg(test)]
    pub fn locomotion(&self) -> &crate::locomotion::Locomotion {
        self.animator.locomotion()
    }

    /// Facing shown, in radians.
    #[cfg(test)]
    pub fn yaw(&self) -> f32 {
        self.animator.yaw()
    }

    /// Where the feet are, in studs.
    #[cfg(test)]
    pub fn feet(&self) -> Vec3 {
        self.feet
    }

    /// Advances by `dt` seconds: the character's root centre is at `root` and faces `target_yaw`
    /// radians; `input` drives the animation (its `turn_rate` is set from the shown turn).
    pub fn update(&mut self, root: Vec3, target_yaw: f32, input: LocomotionInput, dt: f32) {
        self.animator.update(target_yaw, input, dt);
        self.place(root);
    }

    /// Recomputes the joint matrices for a root centre at `root`.
    fn place(&mut self, root: Vec3) {
        self.feet = root - Vec3::new(0.0, ROOT_HEIGHT as f32, 0.0);
        self.model_matrix = model_matrix(self.feet, self.animator.yaw());
        self.animator.pose().skin_matrices(
            &self.animator.model().skeleton,
            self.model_matrix,
            &mut self.joints,
        );
    }

    /// Where the character's body parts are in the pose last drawn, relative to the root centre
    /// `root` (studs). Allocates nothing.
    #[cfg(test)]
    pub fn body_pose(&self, root: Vec3) -> metallic_physics::BodyPose {
        self.animator
            .rig()
            .pose(&self.joints, self.model_matrix, root)
    }

    /// What the renderer draws for this avatar.
    pub fn instance(&self) -> SkinnedInstance<'_> {
        let model = self.animator.model();
        let height = model.height() / STUD_METRES;
        SkinnedInstance {
            model,
            joints: &self.joints,
            slot_colors: &[],
            center: self.feet + Vec3::new(0.0, height * 0.5, 0.0),
            height,
            detail_scale: 1.0 / STUD_METRES,
            style: SkinStyle::Toon,
        }
    }
}
