//! Turns the animated avatar's bones into the character's touch volume pose.
//!
//! Each body part's touch capsule (see [`metallic_physics::BodyPart`]) is carried rigidly by one
//! bone: the part's rest frame is skinned with that bone's matrix, exactly like a mesh vertex
//! weighted 100% to it. [`BodyRig::pose`] does this for the 15 parts from the joint matrices the
//! renderer draws, so the volumes are where the limbs are on screen. It allocates nothing.
use crate::avatar::STUD_METRES;
use metallic_core::{CFrame, Vec3 as CVec3};
use metallic_physics::{BODY_PART_COUNT, BodyPart, BodyPose, ROOT_HEIGHT};
use metallic_render::glam::{Mat4, Quat, Vec3};
use metallic_render::metallic_asset::Skeleton;
use std::f32::consts::PI;

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
pub(crate) fn to_model(p: CVec3) -> Vec3 {
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
    use crate::avatar::{Avatar, default_model};
    use crate::locomotion::{LocomotionInput, Support};
    use metallic_core::{InstanceId, Value, World};
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
    fn computing_the_pose_does_not_allocate() {
        let model = default_model().unwrap();
        let root = Vec3::new(0.0, 3.0, 0.0);
        let avatar = Avatar::new(model, root, 0.3);
        let (pose, allocations) = crate::alloc_count::count(|| {
            let mut last = avatar.body_pose(root);
            for _ in 0..600 {
                last = avatar.body_pose(root);
            }
            last
        });
        assert!(pose.is_finite());
        assert_eq!(allocations, 0);
    }

    /// Drives a standing character through `seconds` with its avatar animated by `input`
    /// (the character itself does not move), handing the bones to the touch volumes each
    /// frame like `Game::frame` does. Returns the names of the parts that touched `target` and
    /// how many TouchEnded events it got.
    fn animate_beside(
        sim: &mut StandingSim,
        input: LocomotionInput,
        seconds: f32,
        target: InstanceId,
    ) -> (Vec<String>, usize) {
        let (mut hits, mut ended) = (Vec::new(), 0);
        for _ in 0..(seconds / DT) as usize {
            sim.avatar.update(sim.root, 0.0, input, DT);
            let pose = sim.avatar.body_pose(sim.root);
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
        avatar: Avatar,
        root: Vec3,
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
        let root = Vec3::new(0.0, 3.0, 0.0);
        let avatar = Avatar::new(default_model().unwrap(), root, 0.0);
        let sim = StandingSim {
            world,
            phys,
            ctl: CharacterController::new(),
            character,
            avatar,
            root,
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
