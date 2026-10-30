//! Two-bone IK, look-at, and root motion extraction.

use metallic_anim::glam::{Quat, Vec2, Vec3};
use metallic_anim::ik::{LookAt, TwoBoneIk, look_at, solve_two_bone};
use metallic_anim::{
    AnimError, Bone2dBuilder, Channel, Clip, Curve, Interpolation, Joint, JointChannel, Keyframe,
    LoopMode, Pose, RootMotionMode, Skeleton, TrackTarget, Transform, root_motion_delta,
    strip_root_motion,
};

/// Shoulder at the origin, upper arm 2 along +X, forearm 1.5 along +X, hand at the end.
fn arm() -> Skeleton {
    let j = |name: &str, parent, x: f32| Joint {
        name: name.into(),
        parent,
        rest: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        length: 0.0,
    };
    Skeleton::new(vec![
        j("Shoulder", None, 0.0),
        j("Elbow", Some(0), 2.0),
        j("Hand", Some(1), 1.5),
    ])
    .unwrap()
}

fn model(s: &Skeleton, p: &Pose) -> Vec<Transform> {
    let mut m = Vec::new();
    p.model_space(s, &mut m);
    m
}

#[test]
fn reaches_a_reachable_target_and_keeps_bone_lengths() {
    let s = arm();
    let mut pose = Pose::rest(&s);
    let target = Vec3::new(1.0, 2.0, 0.5);
    let ik = TwoBoneIk {
        root: 0,
        mid: 1,
        end: 2,
        target,
        pole: Vec3::new(0.0, 0.0, -5.0),
        weight: 1.0,
    };
    solve_two_bone(&s, &mut pose, &ik).unwrap();
    let m = model(&s, &pose);
    assert!(
        m[2].translation.distance(target) < 1e-3,
        "hand at {:?}",
        m[2].translation
    );
    assert!((m[1].translation.distance(m[0].translation) - 2.0).abs() < 1e-4);
    assert!((m[2].translation.distance(m[1].translation) - 1.5).abs() < 1e-4);
    // Elbow bends towards the pole (negative Z side of the shoulder->target line).
    let dir = target.normalize();
    let off = m[1].translation - dir * m[1].translation.dot(dir);
    assert!(off.z < 0.0, "elbow offset {off:?}");
}

#[test]
fn points_straight_at_an_unreachable_target() {
    let s = arm();
    let mut pose = Pose::rest(&s);
    let target = Vec3::new(0.0, 10.0, 0.0);
    let ik = TwoBoneIk {
        root: 0,
        mid: 1,
        end: 2,
        target,
        pole: Vec3::new(1.0, 0.0, 0.0),
        weight: 1.0,
    };
    solve_two_bone(&s, &mut pose, &ik).unwrap();
    let m = model(&s, &pose);
    assert!(
        m[2].translation.normalize().abs_diff_eq(Vec3::Y, 1e-2),
        "{:?}",
        m[2].translation
    );
    assert!((m[2].translation.length() - 3.5).abs() < 1e-2);
}

#[test]
fn zero_weight_changes_nothing_and_bad_chains_error() {
    let s = arm();
    let mut pose = Pose::rest(&s);
    let ik = TwoBoneIk {
        root: 0,
        mid: 1,
        end: 2,
        target: Vec3::Y,
        pole: Vec3::Z,
        weight: 0.0,
    };
    solve_two_bone(&s, &mut pose, &ik).unwrap();
    assert_eq!(pose, Pose::rest(&s));
    let bad = TwoBoneIk {
        root: 0,
        mid: 2,
        end: 1,
        ..ik
    };
    assert_eq!(
        solve_two_bone(&s, &mut pose, &bad),
        Err(AnimError::InvalidChain)
    );
}

#[test]
fn works_on_a_2d_leg_and_stays_in_plane() {
    let mut b = Bone2dBuilder::new();
    let thigh = b.root("Thigh", Vec2::ZERO, -std::f32::consts::FRAC_PI_2, 1.0);
    let shin = b.child("Shin", thigh, 0.0, 1.0);
    b.child("Foot", shin, 0.0, 0.3);
    let s = b.build().unwrap();
    let mut pose = Pose::rest(&s);
    let target = Vec3::new(0.4, -1.5, 0.0);
    let ik = TwoBoneIk {
        root: 0,
        mid: 1,
        end: 2,
        target,
        pole: Vec3::new(5.0, -1.0, 0.0),
        weight: 1.0,
    };
    solve_two_bone(&s, &mut pose, &ik).unwrap();
    let m = model(&s, &pose);
    assert!(m[2].translation.distance(target) < 1e-3);
    assert!(
        m.iter().all(|t| t.translation.z.abs() < 1e-4),
        "stays in the XY plane"
    );
    assert!(m[1].translation.x > 0.0, "knee bends towards the pole");
}

#[test]
fn look_at_turns_and_respects_the_limit() {
    let s = arm();
    let mut pose = Pose::rest(&s);
    let settings = LookAt {
        joint: 0,
        target: Vec3::new(0.0, 5.0, 0.0),
        forward: Vec3::X,
        max_angle: 3.2,
        weight: 1.0,
    };
    look_at(&s, &mut pose, &settings);
    let m = model(&s, &pose);
    assert!((m[0].rotation * Vec3::X).abs_diff_eq(Vec3::Y, 1e-4));

    let mut pose = Pose::rest(&s);
    look_at(
        &s,
        &mut pose,
        &LookAt {
            max_angle: 0.5,
            ..settings
        },
    );
    let m = model(&s, &pose);
    let turned = (m[0].rotation * Vec3::X).angle_between(Vec3::X);
    assert!((turned - 0.5).abs() < 1e-4);
}

fn walk(loop_mode: LoopMode) -> Clip {
    let pos = Curve::from_keys(vec![
        Keyframe::new(0.0, Vec3::new(0.0, 0.0, 0.0)).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Vec3::new(0.0, 0.2, 2.0)).with_interpolation(Interpolation::Linear),
    ])
    .unwrap();
    let mut clip = Clip::new("Walk", 1.0).with_track(
        TrackTarget::joint("Root", JointChannel::Translation),
        Channel::Vec3(pos),
    );
    clip.loop_mode = loop_mode;
    clip
}

#[test]
fn root_motion_accumulates_over_loops() {
    let clip = walk(LoopMode::Loop);
    let d = root_motion_delta(&clip, "Root", RootMotionMode::PlanarXz, 0.25, 0.75);
    assert!(d.translation.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-4));
    // From 0.5 to 2.5: two whole cycles of 2 units.
    let d = root_motion_delta(&clip, "Root", RootMotionMode::PlanarXz, 0.5, 2.5);
    assert!(
        d.translation.abs_diff_eq(Vec3::new(0.0, 0.0, 4.0), 1e-4),
        "{:?}",
        d.translation
    );
    let full = root_motion_delta(&clip, "Root", RootMotionMode::Full, 0.0, 0.5);
    assert!(
        (full.translation.y - 0.1).abs() < 1e-4,
        "full mode keeps height"
    );
    let off = root_motion_delta(&clip, "Root", RootMotionMode::Off, 0.0, 0.5);
    assert_eq!(off.translation, Vec3::ZERO);
    let once = root_motion_delta(
        &walk(LoopMode::Once),
        "Root",
        RootMotionMode::PlanarXz,
        0.5,
        3.0,
    );
    assert!(
        once.translation.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-4),
        "stops at the end"
    );
}

#[test]
fn root_motion_extracts_yaw() {
    let rot = Curve::from_keys(vec![
        Keyframe::new(0.0, Quat::IDENTITY).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Quat::from_rotation_y(1.0) * Quat::from_rotation_x(0.3)),
    ])
    .unwrap();
    let clip = Clip::new("Turn", 1.0).with_track(
        TrackTarget::joint("Root", JointChannel::Rotation),
        Channel::Quat(rot),
    );
    let d = root_motion_delta(&clip, "Root", RootMotionMode::PlanarXz, 0.0, 1.0);
    assert!(
        d.rotation.abs_diff_eq(Quat::from_rotation_y(1.0), 1e-3),
        "{:?}",
        d.rotation
    );
}

#[test]
fn strip_keeps_height_but_removes_planar_travel() {
    let s = Skeleton::new(vec![Joint {
        name: "Root".into(),
        parent: None,
        rest: Transform::IDENTITY,
        length: 0.0,
    }])
    .unwrap();
    let mut pose = Pose::rest(&s);
    pose.local[0].translation = Vec3::new(1.0, 0.3, 2.0);
    pose.local[0].rotation = Quat::from_rotation_y(0.8) * Quat::from_rotation_x(0.2);
    strip_root_motion(&mut pose, &s, 0, RootMotionMode::PlanarXz);
    assert_eq!(pose.local[0].translation, Vec3::new(0.0, 0.3, 0.0));
    assert!(
        pose.local[0]
            .rotation
            .abs_diff_eq(Quat::from_rotation_x(0.2), 1e-4)
    );
}
