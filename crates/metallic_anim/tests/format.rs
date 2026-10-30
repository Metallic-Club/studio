//! Text format: round trips, stable output, one line per key, version and kind checks.

use metallic_anim::format::{
    clip_from_text, clip_to_text, skeleton_from_text, skeleton_to_text, state_machine_from_text,
    state_machine_to_text,
};
use metallic_anim::glam::{Quat, Vec2, Vec3};
use metallic_anim::{
    AnimError, Bone2dBuilder, Channel, Clip, Color3, Condition, Curve, EasingDirection,
    EasingStyle, EventMarker, Interpolation, JointChannel, Keyframe, LoopMode, Motion, Parameter,
    ParameterValue, State, StateMachine, Tangent, TrackTarget, Transition,
};

fn rich_clip() -> Clip {
    let f = Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0f32),
        Keyframe::new(0.5, 1.25).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, -0.5).with_interpolation(Interpolation::Eased(
            EasingStyle::Bounce,
            EasingDirection::Out,
        )),
        Keyframe::new(1.5, 0.0).with_tangents(
            Tangent {
                time: 0.1,
                value: 0.3,
            },
            Tangent {
                time: 0.2,
                value: -0.1,
            },
        ),
    ])
    .unwrap();
    let v2 = Curve::from_keys(vec![
        Keyframe::new(0.0, Vec2::ZERO),
        Keyframe::new(1.0, Vec2::new(1.0, 2.0)),
    ])
    .unwrap();
    let v3 = Curve::from_keys(vec![
        Keyframe::new(0.0, Vec3::ZERO),
        Keyframe::new(2.0, Vec3::new(0.0, 0.5, 4.0)),
    ])
    .unwrap();
    let q = Curve::from_keys(vec![
        Keyframe::new(0.0, Quat::IDENTITY),
        Keyframe::new(1.0, Quat::from_rotation_y(0.7)),
    ])
    .unwrap();
    let col = Curve::from_keys(vec![Keyframe::new(0.0, Color3::from_rgb(155, 130, 212))]).unwrap();
    let b = Curve::from_keys(vec![Keyframe::new(0.0, true), Keyframe::new(0.8, false)]).unwrap();
    let mut clip = Clip::new("Everything", 2.0)
        .with_track(
            TrackTarget::joint("Arm", JointChannel::Rotation),
            Channel::F32(f),
        )
        .with_track(
            TrackTarget::joint("Hip", JointChannel::Translation),
            Channel::Vec2(v2),
        )
        .with_track(
            TrackTarget::joint("Root", JointChannel::Translation),
            Channel::Vec3(v3),
        )
        .with_track(
            TrackTarget::joint("Head", JointChannel::Rotation),
            Channel::Quat(q),
        )
        .with_track(TrackTarget::property("Lamp", "Color"), Channel::Color3(col))
        .with_track(TrackTarget::property("Lamp", "Enabled"), Channel::Bool(b))
        .with_marker(0.5, "Footstep");
    clip.add_marker(EventMarker {
        time: 1.0,
        name: "Sound".into(),
        value: "whoosh".into(),
    });
    clip.loop_mode = LoopMode::PingPong;
    clip.frame_rate = 60.0;
    clip
}

#[test]
fn clip_round_trip_is_equal() {
    let clip = rich_clip();
    let text = clip_to_text(&clip).unwrap();
    let back = clip_from_text(&text).unwrap();
    assert_eq!(back, clip);
}

#[test]
fn save_load_save_gives_identical_text() {
    let text = clip_to_text(&rich_clip()).unwrap();
    let again = clip_to_text(&clip_from_text(&text).unwrap()).unwrap();
    assert_eq!(text, again);
}

#[test]
fn each_key_is_one_line_so_diffs_are_small() {
    let clip = rich_clip();
    let before = clip_to_text(&clip).unwrap();
    assert!(
        before.starts_with("(\n    format: \"metallic-clip\",\n    version: 1,"),
        "{before}"
    );
    let mut edited = clip.clone();
    if let Channel::F32(c) = &mut edited.tracks[0].channel {
        c.key_mut(1).unwrap().value = 2.0;
    }
    let after = clip_to_text(&edited).unwrap();
    let changed: Vec<_> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(before.lines().count(), after.lines().count());
    assert_eq!(changed.len(), 1, "{changed:#?}");
    assert!(
        changed[0].1.contains("time: 0.5") && changed[0].1.contains("value: 2.0"),
        "{changed:?}"
    );
}

#[test]
fn defaults_are_left_out() {
    let c = Curve::from_keys(vec![Keyframe::new(0.0, 0.0f32), Keyframe::new(1.0, 1.0)]).unwrap();
    let clip = Clip::new("Small", 1.0).with_track(
        TrackTarget::joint("A", JointChannel::Rotation),
        Channel::F32(c),
    );
    let text = clip_to_text(&clip).unwrap();
    assert!(!text.contains("tangent_mode"), "{text}");
    assert!(!text.contains("interpolation"), "{text}");
    assert!(!text.contains("markers"), "{text}");
}

#[test]
fn rejects_wrong_kind_newer_version_and_bad_data() {
    let text = clip_to_text(&rich_clip()).unwrap();
    let newer = text.replace("version: 1", "version: 99");
    assert_eq!(
        clip_from_text(&newer),
        Err(AnimError::UnsupportedVersion(99))
    );
    let other = text.replace("metallic-clip", "metallic-skeleton");
    assert!(matches!(clip_from_text(&other), Err(AnimError::Format(_))));
    assert!(matches!(
        clip_from_text("(nonsense"),
        Err(AnimError::Format(_))
    ));
    let bad = text.replace("duration: 2.0", "duration: -1.0");
    assert!(matches!(
        clip_from_text(&bad),
        Err(AnimError::InvalidClip(_))
    ));
}

#[test]
fn skeleton_round_trip_rebuilds_name_index() {
    let mut b = Bone2dBuilder::new();
    let body = b.root("Body", Vec2::new(0.0, 1.0), 0.0, 1.0);
    b.child("Arm", body, 0.5, 0.8);
    let s = b.build().unwrap();
    let back = skeleton_from_text(&skeleton_to_text(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.find("Arm"), Some(1));
}

#[test]
fn state_machine_round_trip() {
    let m = StateMachine {
        parameters: vec![
            Parameter {
                name: "Speed".into(),
                default: ParameterValue::Float(0.0),
            },
            Parameter {
                name: "Jump".into(),
                default: ParameterValue::Trigger,
            },
        ],
        states: vec![
            State {
                name: "Idle".into(),
                motion: Motion::Clip("idle".into()),
                speed: 1.0,
            },
            State {
                name: "Move".into(),
                motion: Motion::Blend1D {
                    parameter: "Speed".into(),
                    points: vec![(0.0, "walk".into()), (8.0, "run".into())],
                },
                speed: 1.0,
            },
        ],
        entry: "Idle".into(),
        transitions: vec![Transition {
            from: Some("Idle".into()),
            to: "Move".into(),
            conditions: vec![Condition::Greater("Speed".into(), 0.1)],
            blend_time: 0.2,
            exit_time: None,
        }],
    };
    let text = state_machine_to_text(&m).unwrap();
    assert_eq!(state_machine_from_text(&text).unwrap(), m);
}
