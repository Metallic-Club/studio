//! The character's body animation on the CPU, and the touch volume pose it gives.
//!
//! [`BodyAnimator`] runs the built-in locomotion over the default avatar's skeleton: clip
//! sampling, blending and the joint hierarchy, with no renderer. The simulation runs one per
//! character every tick and hands [`BodyAnimator::body_pose`] to
//! [`metallic_physics::CharacterController::set_body_pose`], so a headless server computes the
//! same limb poses as a client. The drawn `Avatar` uses the same animator.
//!
//! Each body part's touch capsule (see [`metallic_physics::BodyPart`]) is carried rigidly by one
//! bone: the part's rest frame is skinned with that bone's matrix, exactly like a mesh vertex
//! weighted 100% to it ([`BodyRig::pose`]). Nothing here allocates once warm.
use crate::locomotion::{Locomotion, LocomotionInput, Support};
use glam::{Mat4, Quat, Vec3};
use metallic_asset::{Pose, Skeleton, SkinnedModel};
use metallic_core::{CFrame, InstanceId, Vec3 as CVec3, World};
use metallic_physics::{
    BODY_PART_COUNT, BodyPart, BodyPose, CharacterController, HumanoidState, ROOT_HEIGHT,
};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
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

/// The avatar's model matrix: feet at `feet` (studs), facing `yaw` radians (0 looks along -Z,
/// like `Orientation.Y`; the model itself faces +Z, hence the half turn), metres to studs.
pub(crate) fn model_matrix(feet: Vec3, yaw: f32) -> Mat4 {
    Mat4::from_translation(feet)
        * Mat4::from_rotation_y(yaw + PI)
        * Mat4::from_scale(Vec3::splat(1.0 / STUD_METRES))
}

/// One character's body animation: locomotion state, pose and smoothed facing.
pub(crate) struct BodyAnimator {
    model: Arc<SkinnedModel>,
    locomotion: Locomotion,
    pose: Pose,
    /// Facing, radians around +Y (0 looks along -Z, like `Orientation.Y`).
    yaw: f32,
    rig: BodyRig,
    /// Skinning matrices relative to the root (reused).
    joints: Vec<Mat4>,
}

impl BodyAnimator {
    /// Standing idle, facing `yaw` radians.
    pub fn new(model: Arc<SkinnedModel>, yaw: f32) -> BodyAnimator {
        BodyAnimator {
            locomotion: Locomotion::new(&model),
            pose: Pose::rest(&model.skeleton),
            rig: BodyRig::new(&model.skeleton),
            joints: Vec::with_capacity(model.skeleton.len()),
            yaw: if yaw.is_finite() { yaw } else { 0.0 },
            model,
        }
    }

    /// Advances by `dt` seconds: turns toward `target_yaw` radians and animates from `input`
    /// (its `turn_rate` is set here from the turn).
    pub fn update(&mut self, target_yaw: f32, mut input: LocomotionInput, dt: f32) {
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
    }

    /// The touch volume pose: every body part relative to the root centre.
    pub fn body_pose(&mut self) -> BodyPose {
        let model = model_matrix(Vec3::new(0.0, -(ROOT_HEIGHT as f32), 0.0), self.yaw);
        self.pose
            .skin_matrices(&self.model.skeleton, model, &mut self.joints);
        self.rig.pose(&self.joints, model, Vec3::ZERO)
    }

    /// Facing, in radians.
    #[cfg(feature = "presentation")]
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// The current local joint transforms.
    #[cfg(feature = "presentation")]
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// The model animated.
    #[cfg(feature = "presentation")]
    pub fn model(&self) -> &Arc<SkinnedModel> {
        &self.model
    }

    /// Which bone carries each body part.
    #[cfg(all(test, feature = "presentation"))]
    pub fn rig(&self) -> &BodyRig {
        &self.rig
    }

    /// The animation controller (for tests and diagnostics).
    #[cfg(all(test, feature = "presentation"))]
    pub fn locomotion(&self) -> &Locomotion {
        &self.locomotion
    }
}

/// What the locomotion reads for `character` after its last step: its own speed, vertical speed
/// and ground state.
pub(crate) fn locomotion_input(
    controller: &CharacterController,
    character: InstanceId,
) -> LocomotionInput {
    let support = match controller.state(character) {
        Some(HumanoidState::Running) | None => Support::Grounded,
        Some(HumanoidState::Jumping) => Support::Rising,
        Some(HumanoidState::Freefall) => Support::Falling,
        Some(HumanoidState::Dead) => Support::Frozen,
    };
    let v = controller.relative_velocity(character).unwrap_or_default();
    let speed = (v.x * v.x + v.z * v.z).sqrt() as f32;
    LocomotionInput {
        vertical: v.y as f32,
        ..LocomotionInput::new(speed, support)
    }
}

/// The body animators of the characters a simulation steps.
#[derive(Default)]
pub(crate) struct CharacterBodies {
    animators: HashMap<InstanceId, BodyAnimator>,
}

impl CharacterBodies {
    /// Call after `controller` stepped `character` by `dt` seconds: advances its body animation
    /// and sets the touch volume pose its next step uses. Animators of destroyed characters are
    /// dropped; nothing is allocated once a character has its animator.
    pub fn animate(
        &mut self,
        controller: &mut CharacterController,
        world: &World,
        character: InstanceId,
        dt: f32,
    ) {
        self.animators.retain(|&id, _| world.is_alive(id));
        let Some(yaw) = controller.yaw(character) else {
            return;
        };
        let yaw = (yaw as f32).to_radians();
        let animator = match self.animators.entry(character) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let Some(model) = default_model() else {
                    return;
                };
                e.insert(BodyAnimator::new(model, yaw))
            }
        };
        animator.update(yaw, locomotion_input(controller, character), dt);
        controller.set_body_pose(character, &animator.body_pose());
    }
}

/// The bone that carries each body part, best first; the first name the skeleton has wins.
const BONES: [&[&str]; BODY_PART_COUNT] = [
    &["head", "neck"],
    &["chest", "upperChest", "spine", "hips"],
    &["hips"],
    &["leftUpperArm"],
    &["leftLowerArm"],
    &["leftHand"],
    &["rightUpperArm"],
    &["rightLowerArm"],
    &["rightHand"],
    &["leftUpperLeg"],
    &["leftLowerLeg"],
    &["leftFoot"],
    &["rightUpperLeg"],
    &["rightLowerLeg"],
    &["rightFoot"],
];

/// A body-frame point (studs from the root centre, facing -Z) in the model's rest space (metres
/// from the feet, facing +Z).
fn to_model(p: CVec3) -> Vec3 {
    Vec3::new(-p.x as f32, (p.y + ROOT_HEIGHT) as f32, -p.z as f32) * STUD_METRES
}

/// Which bone carries each body part and where the part's centre sits in the rest model.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BodyRig {
    bones: [Option<usize>; BODY_PART_COUNT],
    centres: [Vec3; BODY_PART_COUNT],
}

impl BodyRig {
    /// Looks the bones up by their VRM names. A part whose bones are all missing keeps its rest
    /// place relative to the avatar.
    pub fn new(skeleton: &Skeleton) -> BodyRig {
        BodyRig {
            bones: BONES.map(|names| names.iter().find_map(|n| skeleton.joint(n))),
            centres: BodyPart::ALL.map(|p| to_model(p.rest_centre())),
        }
    }

    /// The pose of every body part relative to `root` (studs), from the skinning matrices
    /// `joints` (model rest space to world studs). `model` is the avatar's own model matrix,
    /// used for parts without a bone or when `joints` is short.
    pub fn pose(&self, joints: &[Mat4], model: Mat4, root: Vec3) -> BodyPose {
        // The rest frame faces -Z, the model faces +Z: a half turn between them.
        let half_turn = Quat::from_rotation_y(PI);
        let mut pose = BodyPose::rest(0.0);
        for (i, part) in pose.parts.iter_mut().enumerate() {
            let m = self.bones[i]
                .and_then(|j| joints.get(j))
                .copied()
                .unwrap_or(model);
            let at = m.transform_point3(self.centres[i]) - root;
            let (_, rotation, _) = m.to_scale_rotation_translation();
            let q = (rotation * half_turn).normalize();
            *part = CFrame::from_quaternion(
                CVec3::new(f64::from(at.x), f64::from(at.y), f64::from(at.z)),
                [q.x, q.y, q.z, q.w].map(f64::from),
            );
        }
        pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "presentation")]
    use crate::avatar::Avatar;
    use metallic_core::Value;
    use metallic_physics::{
        CharacterController, CharacterEvent, MoveInput, PhysicsWorld, spawn_character,
    };

    const DT: f32 = 1.0 / 60.0;

    fn close(a: CVec3, b: CVec3, tol: f64) -> bool {
        (a - b).magnitude() < tol
    }

    /// Rest position of a joint in the model, in metres.
    fn joint_rest(skeleton: &Skeleton, name: &str) -> Vec3 {
        let j = skeleton.joint(name).unwrap_or_else(|| panic!("no {name}"));
        skeleton.inverse_bind[j]
            .inverse()
            .transform_point3(Vec3::ZERO)
    }

    #[test]
    fn rest_table_matches_the_avatar_joints() {
        let model = default_model().expect("built-in avatar");
        let s = &model.skeleton;
        // (part, which end, joint at that end)
        let ends: [(BodyPart, bool, &str); 16] = [
            (BodyPart::LeftUpperArm, false, "leftUpperArm"),
            (BodyPart::LeftUpperArm, true, "leftLowerArm"),
            (BodyPart::LeftLowerArm, true, "leftHand"),
            (BodyPart::RightUpperArm, false, "rightUpperArm"),
            (BodyPart::RightUpperArm, true, "rightLowerArm"),
            (BodyPart::RightLowerArm, true, "rightHand"),
            (BodyPart::LeftUpperLeg, false, "leftUpperLeg"),
            (BodyPart::LeftUpperLeg, true, "leftLowerLeg"),
            (BodyPart::LeftLowerLeg, true, "leftFoot"),
            (BodyPart::RightUpperLeg, false, "rightUpperLeg"),
            (BodyPart::RightUpperLeg, true, "rightLowerLeg"),
            (BodyPart::RightLowerLeg, true, "rightFoot"),
            (BodyPart::LeftHand, false, "leftHand"),
            (BodyPart::RightHand, false, "rightHand"),
            (BodyPart::LeftLowerArm, false, "leftLowerArm"),
            (BodyPart::RightLowerLeg, false, "rightLowerLeg"),
        ];
        for (part, far, joint) in ends {
            let (a, b) = part.rest_ends();
            let end = to_model(if far { b } else { a });
            let at = joint_rest(s, joint);
            assert!(
                end.distance(at) < 0.01,
                "{part:?} end vs {joint}: {end} {at}"
            );
        }
        // The head volume tops out at the model's height.
        let (_, top) = BodyPart::Head.rest_ends();
        let top = to_model(top).y + BodyPart::Head.radius() as f32 * STUD_METRES;
        assert!(
            (top - model.height()).abs() < 0.03,
            "{top} vs {}",
            model.height()
        );
        // Every part has its own bone.
        let rig = BodyRig::new(s);
        assert!(rig.bones.iter().all(Option::is_some));
    }

    #[cfg(feature = "presentation")]
    #[test]
    fn rest_avatar_gives_the_rest_pose() {
        let model = default_model().unwrap();
        let root = Vec3::new(4.0, 3.0, -2.0);
        for yaw in [0.0f32, 1.0, -2.5] {
            let avatar = Avatar::new(model.clone(), root, yaw);
            let got = avatar.body_pose(root);
            let want = BodyPose::rest(f64::from(yaw).to_degrees());
            for p in BodyPart::ALL {
                let (g, w) = (got.parts[p.index()], want.parts[p.index()]);
                assert!(close(g.position, w.position, 1e-3), "{p:?} {yaw}");
                let probe = CVec3::new(0.3, 0.7, -0.2);
                assert!(
                    close(g.vector_to_world(probe), w.vector_to_world(probe), 1e-4),
                    "{p:?} rotation at yaw {yaw}"
                );
            }
        }
    }

    /// How far a body part's centre moves around (largest minus smallest coordinate, per axis)
    /// over `seconds` of animation driven by `input`, relative to a still root.
    #[cfg(feature = "presentation")]
    fn sweep(input: LocomotionInput, part: BodyPart, seconds: f32) -> (CVec3, CVec3) {
        let model = default_model().unwrap();
        let root = Vec3::new(0.0, 3.0, 0.0);
        let mut avatar = Avatar::new(model, root, 0.0);
        let mut lo = CVec3::new(f64::MAX, f64::MAX, f64::MAX);
        let mut hi = CVec3::new(f64::MIN, f64::MIN, f64::MIN);
        // Settle into the state first, then measure.
        for _ in 0..30 {
            avatar.update(root, 0.0, input, DT);
        }
        for _ in 0..(seconds / DT) as usize {
            avatar.update(root, 0.0, input, DT);
            let c = avatar.body_pose(root).parts[part.index()].position;
            lo = CVec3::new(lo.x.min(c.x), lo.y.min(c.y), lo.z.min(c.z));
            hi = CVec3::new(hi.x.max(c.x), hi.y.max(c.y), hi.z.max(c.z));
        }
        (lo, hi)
    }

    #[cfg(feature = "presentation")]
    #[test]
    fn volumes_follow_the_run_clip() {
        let idle = sweep(
            LocomotionInput::new(0.0, Support::Grounded),
            BodyPart::RightHand,
            1.5,
        );
        let run = sweep(
            LocomotionInput::new(24.0, Support::Grounded),
            BodyPart::RightHand,
            1.5,
        );
        let reach = |(lo, hi): (CVec3, CVec3)| hi.z - lo.z;
        // Facing -Z, the running arm swings forward and back along Z; idle barely moves.
        assert!(reach(run) > 0.8, "run swing {}", reach(run));
        assert!(reach(idle) < 0.3, "idle sway {}", reach(idle));
        let feet = sweep(
            LocomotionInput::new(24.0, Support::Grounded),
            BodyPart::LeftFoot,
            1.5,
        );
        assert!(
            feet.1.y - feet.0.y > 0.5,
            "the running foot lifts: {feet:?}"
        );
    }

    #[cfg(feature = "presentation")]
    #[test]
    fn volumes_follow_the_jump_clip() {
        let idle = sweep(
            LocomotionInput::new(0.0, Support::Grounded),
            BodyPart::LeftFoot,
            0.25,
        );
        let jump = sweep(
            LocomotionInput::new(0.0, Support::Rising),
            BodyPart::LeftFoot,
            0.25,
        );
        // In the jump the legs tuck: the foot rises toward the root.
        assert!(
            jump.1.y > idle.1.y + 0.2,
            "jump foot top {} vs idle {}",
            jump.1.y,
            idle.1.y
        );
    }

    #[test]
    fn animating_and_computing_the_pose_do_not_allocate() {
        // Per frame, in every locomotion state. (A state change inside `metallic_anim`'s state
        // machine may allocate once; that is outside the per-frame path measured here.)
        let model = default_model().unwrap();
        let mut anim = BodyAnimator::new(model, 0.3);
        for input in [
            LocomotionInput::new(0.0, Support::Grounded),
            LocomotionInput::new(24.0, Support::Grounded),
            LocomotionInput::new(0.0, Support::Rising),
            LocomotionInput::new(8.0, Support::Falling),
        ] {
            for _ in 0..90 {
                anim.update(0.5, input, DT);
                anim.body_pose();
            }
            let (pose, allocations) = crate::alloc_count::count(|| {
                let mut last = anim.body_pose();
                for _ in 0..300 {
                    anim.update(0.5, input, DT);
                    last = anim.body_pose();
                }
                last
            });
            assert!(pose.is_finite());
            assert_eq!(allocations, 0, "{input:?}");
        }
    }

    #[test]
    fn the_cpu_pose_does_not_depend_on_the_tick_rate() {
        // The same 1.5 s of running at 30, 60 and 144 Hz: the arm swings just as far and the
        // poses after the same time agree closely.
        let run = LocomotionInput::new(24.0, Support::Grounded);
        let animate = |hz: f32| {
            let mut anim = BodyAnimator::new(default_model().unwrap(), 0.0);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for _ in 0..(1.5 * hz).round() as usize {
                anim.update(0.0, run, 1.0 / hz);
                let pose = anim.body_pose();
                assert!(pose.is_finite());
                let z = pose.parts[BodyPart::RightHand.index()].position.z;
                (lo, hi) = (lo.min(z), hi.max(z));
            }
            (hi - lo, anim.body_pose())
        };
        let (swing60, pose60) = animate(60.0);
        for hz in [30.0, 144.0] {
            let (swing, pose) = animate(hz);
            assert!(
                (swing - swing60).abs() < 0.2,
                "{hz} Hz swing {swing} vs {swing60}"
            );
            for p in BodyPart::ALL {
                let (a, b) = (
                    pose.parts[p.index()].position,
                    pose60.parts[p.index()].position,
                );
                assert!(close(a, b, 0.25), "{p:?} at {hz} Hz: {a:?} vs {b:?}");
            }
        }
    }

    #[cfg(feature = "presentation")]
    #[test]
    fn the_headless_animator_poses_the_body_like_the_drawn_avatar() {
        let model = default_model().unwrap();
        let root = Vec3::new(3.0, 7.0, -1.0);
        let mut avatar = Avatar::new(model.clone(), root, 0.4);
        let mut anim = BodyAnimator::new(model, 0.4);
        for k in 0..240 {
            let input = if k < 120 {
                LocomotionInput::new(24.0, Support::Grounded)
            } else {
                LocomotionInput::new(0.0, Support::Rising)
            };
            avatar.update(root, 1.2, input, DT);
            anim.update(1.2, input, DT);
            let (drawn, headless) = (avatar.body_pose(root), anim.body_pose());
            for p in BodyPart::ALL {
                let (a, b) = (drawn.parts[p.index()], headless.parts[p.index()]);
                assert!(close(a.position, b.position, 1e-3), "{p:?} at frame {k}");
            }
        }
    }

    /// Drives a standing character through `seconds` with its body animated by `input` (the
    /// character itself does not move) on the headless animator, handing the pose to the touch
    /// volumes each tick. Returns the names of the parts that touched `target` and
    /// how many TouchEnded events it got.
    fn animate_beside(
        sim: &mut StandingSim,
        input: LocomotionInput,
        seconds: f32,
        target: InstanceId,
    ) -> (Vec<String>, usize) {
        let (mut hits, mut ended) = (Vec::new(), 0);
        for _ in 0..(seconds / DT) as usize {
            sim.anim.update(0.0, input, DT);
            let pose = sim.anim.body_pose();
            sim.ctl.set_body_pose(sim.character, &pose);
            sim.ctl.step(
                &mut sim.world,
                &sim.phys,
                sim.character,
                MoveInput::default(),
                1.0 / 60.0,
            );
            for e in sim.ctl.drain_events() {
                match e {
                    CharacterEvent::Touched { part, hit, .. } if part == target => {
                        if let Ok(Value::String(n)) = sim.world.get(hit, "Name") {
                            hits.push(n);
                        }
                    }
                    CharacterEvent::TouchEnded { part, .. } if part == target => ended += 1,
                    _ => {}
                }
            }
            sim.phys.step(&mut sim.world, 1.0 / 60.0);
        }
        (hits, ended)
    }

    struct StandingSim {
        world: World,
        phys: PhysicsWorld,
        ctl: CharacterController,
        character: InstanceId,
        anim: BodyAnimator,
    }

    /// A character standing at the origin on a baseplate, and a small sensor at `at`.
    fn standing_beside(at: CVec3) -> (StandingSim, InstanceId) {
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let mut part = |pos: CVec3, size: CVec3, collide: bool| {
            let id = world.create("Part", Some(ws)).unwrap();
            world.set(id, "Position", Value::Vector3(pos)).unwrap();
            world.set(id, "Size", Value::Vector3(size)).unwrap();
            world.set(id, "Anchored", Value::Bool(true)).unwrap();
            world.set(id, "CanCollide", Value::Bool(collide)).unwrap();
            id
        };
        part(
            CVec3::new(0.0, -2.0, 0.0),
            CVec3::new(64.0, 4.0, 64.0),
            true,
        );
        let target = part(at, CVec3::new(0.4, 0.4, 0.4), false);
        let players = world.root("Players").unwrap();
        let player = world.create("Player", Some(players)).unwrap();
        let character = spawn_character(&mut world, player).unwrap();
        let mut phys = PhysicsWorld::new();
        phys.sync_from(&world);
        let anim = BodyAnimator::new(default_model().unwrap(), 0.0);
        let sim = StandingSim {
            world,
            phys,
            ctl: CharacterController::new(),
            character,
            anim,
        };
        (sim, target)
    }

    #[test]
    fn swinging_hand_touches_a_part_the_idle_avatar_beside_it_does_not() {
        // Ahead of the right hip, where the running arm swings to (about 1.85 studs forward)
        // and beyond the movement capsule (1.1 studs with the margin).
        let (mut sim, target) = standing_beside(CVec3::new(0.6, 2.7, -1.85));
        let idle = LocomotionInput::new(0.0, Support::Grounded);
        let (hits, _) = animate_beside(&mut sim, idle, 3.0, target);
        assert!(hits.is_empty(), "the idle avatar touched it: {hits:?}");
        let run = LocomotionInput::new(24.0, Support::Grounded);
        let (hits, ended) = animate_beside(&mut sim, run, 2.0, target);
        assert!(hits.iter().any(|h| h == "RightHand"), "{hits:?}");
        assert!(hits.iter().all(|h| h.starts_with("Right")), "{hits:?}");
        // The arm swings back out of it every stride.
        assert!(ended >= 2, "{ended} TouchEnded for {hits:?}");
    }
}
