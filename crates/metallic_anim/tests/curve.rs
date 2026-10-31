//! Keyframe interpolation: constant, linear, Bezier (with handles and tangent modes) and eased
//! segments, for every value type.

use metallic_anim::glam::{Quat, Vec2, Vec3};
use metallic_anim::{
    Channel, ChannelValue, Color3, Curve, EasingDirection, EasingStyle, Interpolation, Keyframe,
    Tangent, TangentMode,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn two_keys(interp: Interpolation) -> Curve<f32> {
    Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0).with_interpolation(interp),
        Keyframe::new(2.0, 10.0),
    ])
    .unwrap()
}

#[test]
fn constant_holds_until_next_key() {
    let c = two_keys(Interpolation::Constant);
    assert_eq!(c.sample(0.0), Some(0.0));
    assert_eq!(c.sample(1.999), Some(0.0));
    assert_eq!(c.sample(2.0), Some(10.0));
}

#[test]
fn linear_is_a_straight_line() {
    let c = two_keys(Interpolation::Linear);
    assert!(close(c.sample(0.5).unwrap(), 2.5));
    assert!(close(c.sample(1.0).unwrap(), 5.0));
    assert!(close(c.sample(1.5).unwrap(), 7.5));
}

#[test]
fn holds_before_first_and_after_last_key() {
    let c = two_keys(Interpolation::Linear);
    assert_eq!(c.sample(-1.0), Some(0.0));
    assert_eq!(c.sample(99.0), Some(10.0));
    assert_eq!(Curve::<f32>::new().sample(0.0), None);
}

#[test]
fn eased_segment_uses_the_easing_curve() {
    let c = two_keys(Interpolation::Eased(EasingStyle::Quad, EasingDirection::In));
    // Quad In at 0.5 = 0.25.
    assert!(close(c.sample(1.0).unwrap(), 2.5));
    let c = two_keys(Interpolation::Eased(
        EasingStyle::Back,
        EasingDirection::Out,
    ));
    // Back Out at 0.5 = 1.0877 -> overshoots past 10.
    assert!(close(c.sample(1.0).unwrap(), 10.877), "{:?}", c.sample(1.0));
}

#[test]
fn bezier_with_flat_handles_is_smoothstep() {
    // Two keys, auto-clamped: both ends are flat with handles of a third of the segment, so the
    // time axis is linear and the value is 3s^2 - 2s^3.
    let c = Curve::from_keys(vec![Keyframe::new(0.0, 0.0f32), Keyframe::new(1.0, 1.0)]).unwrap();
    for (t, want) in [(0.25, 0.15625), (0.5, 0.5), (0.75, 0.84375)] {
        assert!(
            close(c.sample(t).unwrap(), want),
            "t={t} got {:?}",
            c.sample(t)
        );
    }
}

#[test]
fn auto_tangents_through_collinear_keys_give_a_straight_line() {
    let keys = (0..3)
        .map(|i| Keyframe::new(i as f32, i as f32).with_tangent_mode(TangentMode::Auto))
        .collect();
    let c = Curve::from_keys(keys).unwrap();
    for i in 0..=20 {
        let t = i as f32 / 10.0;
        assert!(
            close(c.sample(t).unwrap(), t),
            "t={t} got {:?}",
            c.sample(t)
        );
    }
}

#[test]
fn auto_clamped_never_overshoots_a_peak() {
    let c = Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0f32),
        Keyframe::new(1.0, 1.0),
        Keyframe::new(2.0, 0.2),
    ])
    .unwrap();
    for i in 0..=200 {
        let v = c.sample(i as f32 / 100.0).unwrap();
        assert!((-1e-5..=1.0 + 1e-5).contains(&v), "overshoot {v}");
    }
    let mut auto = c.clone();
    auto.key_mut(1).unwrap().tangent_mode = TangentMode::Auto;
    auto.recompute_tangents();
    // Plain Auto is allowed to overshoot here; it is smooth through the peak instead.
    let peak = (0..=200)
        .map(|i| auto.sample(i as f32 / 100.0).unwrap())
        .fold(f32::MIN, f32::max);
    assert!(peak > 1.0);
}

#[test]
fn hand_set_handles_shape_the_curve() {
    // Out handle pulls up steeply: early values are above the straight line.
    let a = Keyframe::new(0.0, 0.0f32).with_tangents(
        Tangent::zero(),
        Tangent {
            time: 0.5,
            value: 1.0,
        },
    );
    let b = Keyframe::new(1.0, 1.0f32).with_tangents(
        Tangent {
            time: 0.3,
            value: 0.0,
        },
        Tangent::zero(),
    );
    let c = Curve::from_keys(vec![a, b]).unwrap();
    assert!(c.sample(0.25).unwrap() > 0.4, "{:?}", c.sample(0.25));
    assert!(close(c.sample(1.0).unwrap(), 1.0));
    // Handles longer than the segment are clamped: still a function of time.
    let a = Keyframe::new(0.0, 0.0f32).with_tangents(
        Tangent::zero(),
        Tangent {
            time: 5.0,
            value: 3.0,
        },
    );
    let c = Curve::from_keys(vec![a, Keyframe::new(1.0, 1.0)]).unwrap();
    assert!(c.keys()[0].out_tangent.time <= 1.0);
    assert!(c.sample(0.5).unwrap().is_finite());
}

#[test]
fn flat_and_linear_tangent_modes() {
    let flat = Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0f32).with_tangent_mode(TangentMode::Flat),
        Keyframe::new(1.0, 2.0).with_tangent_mode(TangentMode::Flat),
    ])
    .unwrap();
    assert_eq!(flat.keys()[0].out_tangent.value, 0.0);
    let lin = Curve::from_keys(vec![
        Keyframe::new(0.0, 0.0f32).with_tangent_mode(TangentMode::Linear),
        Keyframe::new(1.0, 2.0).with_tangent_mode(TangentMode::Linear),
    ])
    .unwrap();
    assert!(close(lin.sample(0.25).unwrap(), 0.5));
}

#[test]
fn vector_color_and_rotation_types_interpolate() {
    let v2 = Curve::from_keys(vec![
        Keyframe::new(0.0, Vec2::ZERO).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Vec2::new(2.0, 4.0)),
    ])
    .unwrap();
    assert_eq!(v2.sample(0.5), Some(Vec2::new(1.0, 2.0)));

    let v3 = Curve::from_keys(vec![
        Keyframe::new(0.0, Vec3::ZERO).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Vec3::new(2.0, 4.0, 6.0)),
    ])
    .unwrap();
    assert!(
        v3.sample(0.5)
            .unwrap()
            .abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5)
    );

    let col = Curve::from_keys(vec![
        Keyframe::new(0.0, Color3::new(0.0, 0.0, 0.0)).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Color3::new(1.0, 0.5, 0.25)),
    ])
    .unwrap();
    let c = col.sample(0.5).unwrap();
    assert!(close(c.r, 0.5) && close(c.g, 0.25) && close(c.b, 0.125));

    let q = Curve::from_keys(vec![
        Keyframe::new(0.0, Quat::IDENTITY).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
    ])
    .unwrap();
    let half = q.sample(0.5).unwrap();
    assert!(half.abs_diff_eq(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), 1e-5));
    // Bezier rotation with flat handles: symmetric ease, exactly halfway at the midpoint.
    let qb = Curve::from_keys(vec![
        Keyframe::new(0.0, Quat::IDENTITY),
        Keyframe::new(1.0, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
    ])
    .unwrap();
    assert!(qb.sample(0.5).unwrap().abs_diff_eq(half, 1e-4));
}

#[test]
fn rotation_takes_the_short_way() {
    let a = Quat::from_rotation_z(0.1);
    let b = -Quat::from_rotation_z(0.3); // same rotation, opposite hemisphere
    let c = Curve::from_keys(vec![
        Keyframe::new(0.0, a).with_interpolation(Interpolation::Linear),
        Keyframe::new(1.0, b),
    ])
    .unwrap();
    let mid = c.sample(0.5).unwrap();
    assert!(mid.angle_between(Quat::from_rotation_z(0.2)) < 1e-3);
}

#[test]
fn bool_always_steps() {
    let c = Curve::from_keys(vec![Keyframe::new(0.0, false), Keyframe::new(1.0, true)]).unwrap();
    assert_eq!(c.sample(0.99), Some(false));
    assert_eq!(c.sample(1.0), Some(true));
}

#[test]
fn insert_replaces_same_time_and_sort_remaps() {
    let mut c = two_keys(Interpolation::Linear);
    let i = c.insert(Keyframe::new(1.0, 99.0)).unwrap();
    assert_eq!(i, 1);
    assert_eq!(c.len(), 3);
    c.insert(Keyframe::new(1.0, 5.0)).unwrap();
    assert_eq!(c.len(), 3);
    assert_eq!(c.keys()[1].value, 5.0);
    assert!(c.insert(Keyframe::new(f32::NAN, 0.0)).is_err());
    assert!(c.insert(Keyframe::new(-1.0, 0.0)).is_err());

    // Move key 0 past key 2.
    c.key_mut(0).unwrap().time = 3.0;
    let remap = c.sort();
    assert_eq!(remap, vec![2, 0, 1]);
    assert!(c.validate().is_ok());
}

#[test]
fn channel_helpers_move_and_insert_keys() {
    let mut ch = Channel::F32(two_keys(Interpolation::Linear));
    assert_eq!(ch.key_times(), vec![0.0, 2.0]);
    let i = ch.insert_key_at(1.0).unwrap();
    assert_eq!(ch.key_count(), 3);
    // Inserting a key does not change the curve's value there.
    assert!(matches!(ch.sample(1.0), Some(ChannelValue::F32(v)) if close(v, 5.0)));
    let remap = ch.set_key_times(&[(i, 3.0)]);
    assert_eq!(remap[i], 2);
    assert_eq!(ch.key_times(), vec![0.0, 2.0, 3.0]);
}
