//! Tween timing: TweenService-style repeat, reverse and delay semantics.

use metallic_anim::glam::Vec3;
use metallic_anim::{EasingDirection, EasingStyle, PlaybackState, Tween, TweenInfo};

fn linear(time: f32) -> TweenInfo {
    TweenInfo::new(time).with_easing(EasingStyle::Linear, EasingDirection::In)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn defaults_match_tweeninfo_new() {
    let i = TweenInfo::default();
    assert_eq!(i.time, 1.0);
    assert_eq!(i.easing_style, EasingStyle::Quad);
    assert_eq!(i.easing_direction, EasingDirection::Out);
    assert_eq!(i.repeat_count, 0);
    assert!(!i.reverses);
    assert_eq!(i.delay_time, 0.0);
}

#[test]
fn single_pass_reaches_goal_and_completes() {
    let i = linear(2.0);
    assert_eq!(i.total_duration(), Some(2.0));
    assert!(close(i.sample(1.0).alpha, 0.5));
    let end = i.sample(2.0);
    assert!(end.finished && end.alpha == 1.0);
}

#[test]
fn reverses_doubles_the_cycle_and_ends_at_start() {
    let i = linear(1.0).with_repeat(0, true);
    assert_eq!(i.total_duration(), Some(2.0));
    assert!(close(i.sample(0.5).alpha, 0.5));
    assert!(close(i.sample(1.0).alpha, 1.0));
    assert!(close(i.sample(1.5).alpha, 0.5));
    let end = i.sample(2.0);
    assert!(end.finished && end.alpha == 0.0);
}

#[test]
fn repeat_count_adds_cycles() {
    let i = linear(1.0).with_repeat(2, false);
    assert_eq!(i.total_duration(), Some(3.0));
    assert!(
        close(i.sample(1.25).alpha, 0.25),
        "second cycle restarts from the start"
    );
    assert!(!i.sample(2.9).finished);
    assert!(i.sample(3.0).finished);

    let i = linear(1.0).with_repeat(2, true);
    assert_eq!(i.total_duration(), Some(6.0));
    assert!(close(i.sample(3.5).alpha, 0.5)); // cycle 2, going back
    assert!(close(i.sample(4.25).alpha, 0.25)); // cycle 3, going forward
}

#[test]
fn negative_repeat_count_is_forever() {
    let i = linear(1.0).with_repeat(-1, true);
    assert_eq!(i.total_duration(), None);
    let s = i.sample(1_000_000.5);
    assert!(!s.finished);
    assert!(close(i.sample(1001.25).alpha, 0.75));
}

#[test]
fn delay_happens_once_before_the_first_cycle() {
    let i = linear(1.0).with_delay(0.5).with_repeat(1, false);
    assert_eq!(i.total_duration(), Some(2.5));
    assert_eq!(i.sample(0.25).alpha, 0.0);
    assert!(close(i.sample(1.0).alpha, 0.5));
    assert!(
        close(i.sample(1.75).alpha, 0.25),
        "no second delay between cycles"
    );
}

#[test]
fn easing_is_applied_per_pass() {
    let i = TweenInfo::new(1.0)
        .with_easing(EasingStyle::Quad, EasingDirection::In)
        .with_repeat(0, true);
    assert!(close(i.sample(0.5).alpha, 0.25));
    // Going back mirrors the forward pass.
    assert!(close(i.sample(1.5).alpha, 0.25));
}

#[test]
fn tween_object_plays_pauses_cancels() {
    let mut t = Tween::new(
        linear(1.0).with_delay(0.2),
        Vec3::ZERO,
        Vec3::new(10.0, 0.0, 0.0),
    );
    assert_eq!(t.state(), PlaybackState::Begin);
    assert_eq!(t.update(0.5), Vec3::ZERO, "not played yet");
    t.play();
    assert_eq!(t.state(), PlaybackState::Delayed);
    t.update(0.1);
    assert_eq!(t.state(), PlaybackState::Delayed);
    let v = t.update(0.6);
    assert_eq!(t.state(), PlaybackState::Playing);
    assert!(close(v.x, 5.0));
    t.pause();
    assert_eq!(t.update(10.0), v, "paused tweens do not move");
    t.play();
    t.update(0.25);
    assert!(close(t.value().x, 7.5));
    t.cancel();
    assert_eq!(t.state(), PlaybackState::Cancelled);
    assert!(
        close(t.value().x, 7.5),
        "cancel leaves the value where it is"
    );
    t.play();
    assert_eq!(t.elapsed(), 0.0, "play after cancel starts over");
    t.update(5.0);
    assert_eq!(t.state(), PlaybackState::Completed);
    assert_eq!(t.value(), Vec3::new(10.0, 0.0, 0.0));
}

#[test]
fn overshooting_styles_go_past_the_goal() {
    let mut t = Tween::new(
        TweenInfo::new(1.0).with_easing(EasingStyle::Back, EasingDirection::Out),
        0.0f32,
        1.0,
    );
    t.play();
    assert!(t.update(0.5) > 1.0);
}
