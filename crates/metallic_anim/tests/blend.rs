//! Skeletons, sampling, pose blending (crossfade, masks, additive) and the Animator's layering.

use metallic_anim::glam::{Quat, Vec2, Vec3};
use metallic_anim::{
    AnimationPriority, Animator, AnimatorEvent, BlendMode, Bone2dBuilder, Channel, Clip,
    ClipBinding, Curve, Interpolation, Joint, JointChannel, JointMask, Keyframe, LoopMode,
    PlayOptions, Pose, Skeleton, TrackTarget, Transform, sample_clip,
};
use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

fn figure() -> Skeleton {
    let j = |name: &str, parent: Option<usize>, at: Vec3| Joint {
        name: name.to_owned(),
        parent,
        rest: Transform::from_translation(at),
        length: 0.0,
    };
    Skeleton::new(vec![
        j("Root", None, Vec3::new(0.0, 3.0, 0.0)),
        j("Torso", Some(0), Vec3::new(0.0, 1.0, 0.0)),
        j("Head", Some(1), Vec3::new(0.0, 1.5, 0.0)),
        j("LeftArm", Some(1), Vec3::new(-1.5, 1.0, 0.0)),
        j("RightArm", Some(1), Vec3::new(1.5, 1.0, 0.0)),
        j("LeftLeg", Some(0), Vec3::new(-0.5, -1.0, 0.0)),
        j("RightLeg", Some(0), Vec3::new(0.5, -1.0, 0.0)),
    ])
    .unwrap()
}

/// A clip that turns `joint` about X from 0 to `angle` over 1 s (linear).
fn turn(name: &str, joint: &str, angle: f32) -> Clip {
    let c = Curve::from_keys(vec![
        Keyframe::new(0.0, Quat::IDENTITY).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Quat::from_rotation_x(angle)),
    ])
    .unwrap();
    Clip::new(name, 1.0).with_track(
        TrackTarget::joint(joint, JointChannel::Rotation),
        Channel::Quat(c),
    )
}

/// A clip that holds `joint` at a fixed rotation about X.
fn hold(name: &str, joint: &str, angle: f32) -> Clip {
    let c = Curve::from_keys(vec![Keyframe::new(0.0, Quat::from_rotation_x(angle))]).unwrap();
    let mut clip = Clip::new(name, 1.0).with_track(
        TrackTarget::joint(joint, JointChannel::Rotation),
        Channel::Quat(c),
    );
    clip.loop_mode = LoopMode::Loop;
    clip
}

fn angle_x(q: Quat) -> f32 {
    let (axis, angle) = q.to_axis_angle();
    if axis.x < 0.0 { -angle } else { angle }
}

#[test]
fn skeleton_rejects_bad_parents_and_duplicate_names() {
    let j = |name: &str, parent| Joint {
        name: name.into(),
        parent,
        rest: Transform::IDENTITY,
        length: 0.0,
    };
    assert!(Skeleton::new(vec![j("A", Some(0))]).is_err());
    assert!(Skeleton::new(vec![j("A", None), j("A", Some(0))]).is_err());
    assert!(Skeleton::new(vec![j("A", None), j("B", Some(0))]).is_ok());
}

#[test]
fn tracks_are_offsets_from_rest_and_model_space_composes() {
    let s = figure();
    let clip = turn("Bow", "Torso", FRAC_PI_2);
    let b = ClipBinding::new(&clip, &s);
    let mut pose = Pose::rest(&s);
    sample_clip(&clip, &b, &s, 1.0, &mut pose);
    // Torso kept its rest position and got the rotation.
    assert_eq!(pose.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
    assert!((angle_x(pose.local[1].rotation) - FRAC_PI_2).abs() < 1e-4);
    let mut model = Vec::new();
    pose.model_space(&s, &mut model);
    // Head sits 1.5 above the torso along the torso's +Y, which now points along +Z.
    assert!(
        model[2]
            .translation
            .abs_diff_eq(Vec3::new(0.0, 4.0, 1.5), 1e-4),
        "{:?}",
        model[2].translation
    );
}

#[test]
fn unknown_joints_are_reported_not_fatal() {
    let s = figure();
    let clip = turn("Wag", "Tail", 1.0);
    let b = ClipBinding::new(&clip, &s);
    assert_eq!(b.unresolved(), ["Tail".to_owned()]);
    assert!(ClipBinding::new_strict(&clip, &s).is_err());
}

#[test]
fn crossfade_blend_is_halfway_at_half_weight() {
    let s = figure();
    let mut a = Pose::rest(&s);
    let mut b = Pose::rest(&s);
    a.local[3].rotation = Quat::IDENTITY;
    b.local[3].rotation = Quat::from_rotation_x(1.0);
    b.local[3].translation += Vec3::new(0.0, 2.0, 0.0);
    let mut out = Pose::rest(&s);
    Pose::blend(&a, &b, 0.5, None, &mut out);
    assert!((angle_x(out.local[3].rotation) - 0.5).abs() < 1e-4);
    assert!((out.local[3].translation.y - (a.local[3].translation.y + 1.0)).abs() < 1e-5);
    Pose::blend(&a, &b, 0.0, None, &mut out);
    assert_eq!(out, a);
    Pose::blend(&a, &b, 1.0, None, &mut out);
    assert_eq!(out, b);
}

#[test]
fn masks_limit_blending_to_a_subtree() {
    let s = figure();
    let mask = JointMask::subtree(&s, "Torso", 1.0).unwrap();
    for (name, want) in [
        ("Root", 0.0),
        ("Torso", 1.0),
        ("Head", 1.0),
        ("RightArm", 1.0),
        ("LeftLeg", 0.0),
    ] {
        assert_eq!(mask.weight(s.find(name).unwrap()), want, "{name}");
    }
    let a = Pose::rest(&s);
    let mut b = Pose::rest(&s);
    for t in &mut b.local {
        t.rotation = Quat::from_rotation_x(1.0);
    }
    let mut out = Pose::rest(&s);
    Pose::blend(&a, &b, 1.0, Some(&mask), &mut out);
    assert_eq!(out.local[0], a.local[0]);
    assert_eq!(out.local[5], a.local[5]);
    assert_eq!(out.local[4], b.local[4]);
    assert!(JointMask::subtree(&s, "Nope", 1.0).is_err());
}

#[test]
fn additive_round_trip_and_partial_weight() {
    let s = figure();
    let reference = Pose::rest(&s);
    let mut leaned = Pose::rest(&s);
    leaned.local[1].rotation = Quat::from_rotation_z(0.4);
    leaned.local[1].translation.x += 0.2;
    let delta = leaned.additive_delta(&reference);

    // Adding the full delta to the reference reproduces the animated pose.
    let mut p = reference.clone();
    p.add_additive(&delta, 1.0, None);
    assert!(
        p.local[1]
            .rotation
            .abs_diff_eq(leaned.local[1].rotation, 1e-5)
    );
    assert!(
        p.local[1]
            .translation
            .abs_diff_eq(leaned.local[1].translation, 1e-5)
    );

    // On top of another pose, half weight adds half the lean.
    let mut base = Pose::rest(&s);
    base.local[1].rotation = Quat::from_rotation_x(0.3);
    base.add_additive(&delta, 0.5, None);
    let expected = Quat::from_rotation_x(0.3) * Quat::from_rotation_z(0.2);
    assert!(base.local[1].rotation.abs_diff_eq(expected, 1e-5));
    assert!((base.local[1].translation.x - 0.1).abs() < 1e-5);
}

#[test]
fn two_d_bones_chain_along_parent_x() {
    let mut b = Bone2dBuilder::new();
    let root = b.root("Upper", Vec2::ZERO, 0.0, 2.0);
    b.child("Lower", root, FRAC_PI_2, 1.0);
    let s = b.build().unwrap();
    let mut model = Vec::new();
    Pose::rest(&s).model_space(&s, &mut model);
    assert!(
        model[1]
            .translation
            .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5)
    );
    // Lower bone tip = its origin + rotated +X * length.
    let tip = model[1].transform_point(Vec3::new(1.0, 0.0, 0.0));
    assert!(tip.abs_diff_eq(Vec3::new(2.0, 1.0, 0.0), 1e-5));
    let (_, angle, _) = model[1].to_2d();
    assert!((angle - FRAC_PI_2).abs() < 1e-5);
}

#[test]
fn f32_rotation_track_drives_a_2d_bone_angle() {
    let mut b = Bone2dBuilder::new();
    b.root("Arm", Vec2::ZERO, 0.0, 1.0);
    let s = b.build().unwrap();
    let c = Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0f32).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, 1.0),
    ])
    .unwrap();
    let clip = Clip::new("Swing", 1.0).with_track(
        TrackTarget::joint("Arm", JointChannel::Rotation),
        Channel::F32(c),
    );
    let mut pose = Pose::rest(&s);
    sample_clip(&clip, &ClipBinding::new(&clip, &s), &s, 0.5, &mut pose);
    assert!((pose.local[0].to_2d().1 - 0.5).abs() < 1e-5);
}

#[test]
fn animator_crossfades_linearly() {
    let s = Arc::new(figure());
    let mut anim = Animator::new(Arc::clone(&s));
    let a = anim.load(Arc::new(hold("A", "LeftArm", 0.0)));
    let b = anim.load(Arc::new(hold("B", "LeftArm", 1.0)));
    anim.play(
        a,
        PlayOptions {
            fade_time: 0.0,
            ..PlayOptions::default()
        },
    );
    anim.update(0.1);
    anim.crossfade(a, b, 0.5);
    anim.update(0.25);
    assert!((anim.weight(a).unwrap() - 0.5).abs() < 1e-5);
    assert!((anim.weight(b).unwrap() - 0.5).abs() < 1e-5);
    let mut pose = Pose::rest(&s);
    anim.evaluate(&mut pose);
    assert!(
        (angle_x(pose.local[3].rotation) - 0.5).abs() < 1e-3,
        "{}",
        angle_x(pose.local[3].rotation)
    );
    let events = anim.update(0.3);
    assert!(events.contains(&AnimatorEvent::Stopped(a)));
    assert!(!anim.is_playing(a));
    anim.evaluate(&mut pose);
    assert!((angle_x(pose.local[3].rotation) - 1.0).abs() < 1e-3);
}

#[test]
fn higher_priority_overrides_only_the_joints_it_animates() {
    let s = Arc::new(figure());
    let mut anim = Animator::new(Arc::clone(&s));
    let mut walk = hold("Walk", "LeftArm", 0.4);
    walk.tracks.extend(hold("x", "LeftLeg", 0.6).tracks);
    let walk = anim.load(Arc::new(walk));
    let wave = anim.load(Arc::new(hold("Wave", "LeftArm", 1.2)));
    anim.set_priority(walk, AnimationPriority::Movement);
    anim.set_priority(wave, AnimationPriority::Action);
    let now = PlayOptions {
        fade_time: 0.0,
        ..PlayOptions::default()
    };
    anim.play(walk, now);
    anim.play(wave, now);
    anim.update(0.1);
    let mut pose = Pose::rest(&s);
    anim.evaluate(&mut pose);
    assert!(
        (angle_x(pose.local[3].rotation) - 1.2).abs() < 1e-4,
        "arm comes from Action"
    );
    assert!(
        (angle_x(pose.local[5].rotation) - 0.6).abs() < 1e-4,
        "leg still walks"
    );

    // Same priority: averaged by weight.
    anim.set_priority(wave, AnimationPriority::Movement);
    anim.evaluate(&mut pose);
    assert!((angle_x(pose.local[3].rotation) - 0.8).abs() < 1e-3);
}

#[test]
fn additive_track_adds_on_top() {
    let s = Arc::new(figure());
    let mut anim = Animator::new(Arc::clone(&s));
    let base = anim.load(Arc::new(hold("Base", "Torso", 0.5)));
    let lean = anim.load(Arc::new(turn("Lean", "Torso", 0.2)));
    anim.set_blend_mode(lean, BlendMode::Additive);
    let now = PlayOptions {
        fade_time: 0.0,
        ..PlayOptions::default()
    };
    anim.play(base, now);
    anim.play(lean, now);
    anim.set_time_position(lean, 0.5);
    anim.update(0.0);
    let mut pose = Pose::rest(&s);
    anim.evaluate(&mut pose);
    // 0.5 from the base plus half of the 0.2 lean.
    assert!((angle_x(pose.local[1].rotation) - 0.6).abs() < 1e-3);
}

#[test]
fn markers_fire_once_per_pass_and_once_clips_end() {
    let s = Arc::new(figure());
    let mut anim = Animator::new(Arc::clone(&s));
    let clip = turn("Step", "LeftLeg", 1.0)
        .with_marker(0.0, "Start")
        .with_marker(0.5, "Footstep");
    let h = anim.load(Arc::new(clip));
    anim.play(
        h,
        PlayOptions {
            fade_time: 0.2,
            ..PlayOptions::default()
        },
    );
    let names = |ev: &[AnimatorEvent]| -> Vec<String> {
        ev.iter()
            .filter_map(|e| match e {
                AnimatorEvent::Marker { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(names(&anim.update(0.1)), ["Start"]);
    assert!(names(&anim.update(0.3)).is_empty());
    assert_eq!(names(&anim.update(0.2)), ["Footstep"]);
    let ev = anim.update(0.5);
    assert!(ev.contains(&AnimatorEvent::Ended(h)));
    assert!(anim.is_playing(h), "fades out after the end");
    let ev = anim.update(0.3);
    assert!(ev.contains(&AnimatorEvent::Stopped(h)));
}

#[test]
fn looping_clip_reports_markers_every_loop() {
    let s = Arc::new(figure());
    let mut anim = Animator::new(Arc::clone(&s));
    let mut clip = turn("Run", "LeftLeg", 1.0)
        .with_marker(0.25, "L")
        .with_marker(0.75, "R");
    clip.loop_mode = LoopMode::Loop;
    let h = anim.load(Arc::new(clip));
    anim.play(h, PlayOptions::default());
    let ev = anim.update(2.5);
    let markers = ev
        .iter()
        .filter(|e| matches!(e, AnimatorEvent::Marker { .. }))
        .count();
    assert_eq!(markers, 5); // L R L R L
    assert!(ev.contains(&AnimatorEvent::Looped(h)));
    assert!((anim.time_position(h).unwrap() - 0.5).abs() < 1e-5);
}
