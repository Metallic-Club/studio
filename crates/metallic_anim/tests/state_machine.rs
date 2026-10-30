//! State machine transitions, conditions, triggers, exit times, crossfades and 1D blends.

use metallic_anim::{
    AnimError, Clip, Condition, LoopMode, Motion, Parameter, ParameterValue, State, StateMachine,
    StateMachinePlayer, Transition,
};
use std::sync::Arc;

fn clip(name: &str, duration: f32) -> Arc<Clip> {
    let mut c = Clip::new(name, duration);
    c.loop_mode = LoopMode::Loop;
    Arc::new(c)
}

fn state(name: &str, clip: &str) -> State {
    State {
        name: name.into(),
        motion: Motion::Clip(clip.into()),
        speed: 1.0,
    }
}

fn tr(from: Option<&str>, to: &str, conditions: Vec<Condition>, blend: f32) -> Transition {
    Transition {
        from: from.map(Into::into),
        to: to.into(),
        conditions,
        blend_time: blend,
        exit_time: None,
    }
}

fn machine() -> StateMachine {
    StateMachine {
        parameters: vec![
            Parameter {
                name: "Speed".into(),
                default: ParameterValue::Float(0.0),
            },
            Parameter {
                name: "Grounded".into(),
                default: ParameterValue::Bool(true),
            },
            Parameter {
                name: "Jump".into(),
                default: ParameterValue::Trigger,
            },
        ],
        states: vec![
            state("Idle", "idle"),
            state("Run", "run"),
            state("Jump", "jump"),
            state("Fall", "fall"),
        ],
        entry: "Idle".into(),
        transitions: vec![
            tr(
                Some("Idle"),
                "Run",
                vec![Condition::Greater("Speed".into(), 0.1)],
                0.2,
            ),
            tr(
                Some("Run"),
                "Idle",
                vec![Condition::Less("Speed".into(), 0.1)],
                0.2,
            ),
            tr(
                None,
                "Jump",
                vec![
                    Condition::Triggered("Jump".into()),
                    Condition::Is("Grounded".into(), true),
                ],
                0.1,
            ),
            Transition {
                from: Some("Jump".into()),
                to: "Fall".into(),
                conditions: vec![],
                blend_time: 0.1,
                exit_time: Some(1.0),
            },
            tr(
                Some("Fall"),
                "Idle",
                vec![Condition::Is("Grounded".into(), true)],
                0.0,
            ),
        ],
    }
}

fn player() -> StateMachinePlayer {
    let clips = vec![
        clip("idle", 2.0),
        clip("run", 0.8),
        clip("jump", 0.5),
        clip("fall", 1.0),
    ];
    StateMachinePlayer::new(&machine(), clips).unwrap()
}

#[test]
fn float_conditions_switch_states() {
    let mut p = player();
    assert_eq!(p.current_state(), "Idle");
    assert_eq!(p.update(0.1), None);
    p.set_float("Speed", 5.0).unwrap();
    assert_eq!(p.update(0.016).as_deref(), Some("Run"));
    assert!(p.is_blending());
    p.set_float("Speed", 0.0).unwrap();
    assert_eq!(p.update(0.016).as_deref(), Some("Idle"));
}

#[test]
fn crossfade_weights_sum_to_one_and_finish() {
    let mut p = player();
    p.set_float("Speed", 5.0).unwrap();
    p.update(0.0);
    p.update(0.1); // halfway through a 0.2 s blend
    let s = p.motion_samples();
    assert_eq!(s.len(), 2);
    let total: f32 = s.iter().map(|m| m.weight).sum();
    assert!((total - 1.0).abs() < 1e-5);
    let run = s.iter().find(|m| m.clip == 1).unwrap();
    assert!((run.weight - 0.5).abs() < 1e-5);
    p.update(0.15);
    assert!(!p.is_blending());
    assert_eq!(p.motion_samples().len(), 1);
}

#[test]
fn triggers_fire_once_and_expire() {
    let mut p = player();
    p.fire("Jump").unwrap();
    assert_eq!(p.update(0.016).as_deref(), Some("Jump"));
    // Consumed: does not re-enter Jump from Jump's own "any state" check.
    assert_eq!(p.update(0.016), None);

    // A trigger that cannot fire (not grounded) expires instead of firing much later.
    let mut p = player();
    p.set_bool("Grounded", false).unwrap();
    p.fire("Jump").unwrap();
    assert_eq!(p.update(0.016), None);
    p.set_bool("Grounded", true).unwrap();
    assert_eq!(p.update(0.016), None);
}

#[test]
fn exit_time_waits_for_the_clip_to_finish() {
    let mut p = player();
    p.fire("Jump").unwrap();
    p.update(0.0);
    assert_eq!(p.current_state(), "Jump");
    assert_eq!(p.update(0.4), None); // jump clip is 0.5 s
    assert_eq!(p.update(0.11).as_deref(), Some("Fall"));
}

#[test]
fn any_state_transition_interrupts() {
    let mut p = player();
    p.set_float("Speed", 5.0).unwrap();
    p.update(0.0);
    p.fire("Jump").unwrap();
    assert_eq!(p.update(0.05).as_deref(), Some("Jump"));
}

#[test]
fn blend_1d_picks_neighbours_and_syncs_playheads() {
    let m = StateMachine {
        parameters: vec![Parameter {
            name: "Speed".into(),
            default: ParameterValue::Float(0.0),
        }],
        states: vec![State {
            name: "Locomotion".into(),
            motion: Motion::Blend1D {
                parameter: "Speed".into(),
                points: vec![
                    (0.0, "idle".into()),
                    (4.0, "walk".into()),
                    (12.0, "run".into()),
                ],
            },
            speed: 1.0,
        }],
        entry: "Locomotion".into(),
        transitions: vec![],
    };
    let clips = vec![clip("idle", 2.0), clip("walk", 1.0), clip("run", 0.5)];
    let mut p = StateMachinePlayer::new(&m, clips).unwrap();
    p.set_float("Speed", 8.0).unwrap();
    p.update(0.0);
    let s = p.motion_samples();
    assert_eq!(s.len(), 2);
    assert_eq!((s[0].clip, s[1].clip), (1, 2));
    assert!((s[0].weight - 0.5).abs() < 1e-5 && (s[1].weight - 0.5).abs() < 1e-5);
    // Weighted cycle length 0.75 s; after 0.375 s both clips are at half their length.
    p.update(0.375);
    let s = p.motion_samples();
    assert!((s[0].time - 0.5).abs() < 1e-4, "{s:?}");
    assert!((s[1].time - 0.25).abs() < 1e-4, "{s:?}");
    p.set_float("Speed", 20.0).unwrap();
    assert_eq!(p.motion_samples().len(), 1);
}

#[test]
fn names_are_checked_up_front() {
    let clips = vec![clip("idle", 1.0)];
    let mut m = machine();
    assert!(matches!(
        StateMachinePlayer::new(&m, clips.clone()),
        Err(AnimError::UnknownClip(_))
    ));
    m.states.truncate(1);
    m.transitions = vec![tr(Some("Idle"), "Nope", vec![], 0.0)];
    assert!(matches!(
        StateMachinePlayer::new(&m, clips.clone()),
        Err(AnimError::UnknownState(_))
    ));
    m.transitions = vec![tr(
        Some("Idle"),
        "Idle",
        vec![Condition::Is("Speed".into(), true)],
        0.0,
    )];
    assert!(matches!(
        StateMachinePlayer::new(&m, clips.clone()),
        Err(AnimError::ParameterType(_))
    ));
    m.transitions = vec![tr(
        Some("Idle"),
        "Idle",
        vec![Condition::Triggered("Missing".into())],
        0.0,
    )];
    assert!(matches!(
        StateMachinePlayer::new(&m, clips),
        Err(AnimError::UnknownParameter(_))
    ));
    let mut p = player();
    assert!(p.set_bool("Speed", true).is_err());
    assert!(p.fire("Nope").is_err());
}

#[test]
fn playback_rate_scales_how_fast_a_state_plays() {
    let time_after = |rate: f32| {
        let mut p = player();
        p.set_playback_rate(rate);
        p.update(0.5);
        p.motion_samples()[0].time
    };
    // Idle is 2 s long and loops.
    assert!((time_after(1.0) - 0.5).abs() < 1e-5);
    assert!((time_after(2.0) - 1.0).abs() < 1e-5);
    assert!((time_after(0.5) - 0.25).abs() < 1e-5);
    assert_eq!(time_after(-3.0), 0.0);
    assert_eq!(time_after(f32::NAN), 0.0);
}

#[test]
fn crossfade_eases_with_smoothstep() {
    let mut p = player();
    p.set_float("Speed", 5.0).unwrap();
    p.update(0.0);
    p.update(0.05); // a quarter of the 0.2 s blend
    let run = p
        .motion_samples()
        .into_iter()
        .find(|m| m.clip == 1)
        .unwrap();
    assert!((run.weight - 0.15625).abs() < 1e-5, "{}", run.weight);
}
