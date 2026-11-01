//! Spatial math against closed formulas.

use metallic_audio::{
    DOPPLER_RANGE, Emitter, Listener, OPEN_CUTOFF_HZ, RollOff, RollOffMode, SPEED_OF_SOUND_MPS,
    SpatialSettings, Vec3, doppler_factor, listener_local, pan_gains, roll_off_gain, spatialize,
};

const EPS: f32 = 1e-5;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS * (1.0 + b.abs())
}

#[test]
fn inverse_roll_off_is_min_over_distance() {
    for d in [10.5f32, 12.0, 20.0, 100.0, 777.0, 5_000.0, 9_999.0] {
        let g = roll_off_gain(RollOffMode::Inverse, d, 10.0, 10_000.0);
        assert!(close(g, 10.0 / d), "d={d} g={g}");
    }
}

#[test]
fn every_mode_is_full_inside_min_and_silent_beyond_max() {
    for mode in [
        RollOffMode::Inverse,
        RollOffMode::Linear,
        RollOffMode::LinearSquare,
        RollOffMode::InverseTapered,
    ] {
        for d in [0.0f32, 3.0, 10.0] {
            assert_eq!(roll_off_gain(mode, d, 10.0, 100.0), 1.0, "{mode:?} d={d}");
        }
        for d in [100.0f32, 150.0, 1e9] {
            assert_eq!(roll_off_gain(mode, d, 10.0, 100.0), 0.0, "{mode:?} d={d}");
        }
    }
}

#[test]
fn linear_and_linear_square() {
    // min 10, max 110: d = 60 is halfway.
    assert!(close(
        roll_off_gain(RollOffMode::Linear, 60.0, 10.0, 110.0),
        0.5
    ));
    assert!(close(
        roll_off_gain(RollOffMode::LinearSquare, 60.0, 10.0, 110.0),
        0.25
    ));
    assert!(close(
        roll_off_gain(RollOffMode::Linear, 35.0, 10.0, 110.0),
        0.75
    ));
    assert!(close(
        roll_off_gain(RollOffMode::LinearSquare, 35.0, 10.0, 110.0),
        0.5625
    ));
}

#[test]
fn inverse_tapered_is_the_lower_of_inverse_and_linear_square() {
    let (min, max) = (10.0f32, 200.0f32);
    for i in 1..400 {
        let d = min + (max - min) * i as f32 / 400.0;
        let inverse = min / d;
        let lin = (max - d) / (max - min);
        let expected = inverse.min(lin * lin);
        assert!(
            close(
                roll_off_gain(RollOffMode::InverseTapered, d, min, max),
                expected
            ),
            "d={d}"
        );
    }
    // Near min it follows Inverse, near max LinearSquare.
    assert!(close(
        roll_off_gain(RollOffMode::InverseTapered, 11.0, min, max),
        10.0 / 11.0
    ));
    let near_max = 195.0;
    let lin = (max - near_max) / (max - min);
    assert!(close(
        roll_off_gain(RollOffMode::InverseTapered, near_max, min, max),
        lin * lin
    ));
}

#[test]
fn curves_never_increase_with_distance() {
    for mode in [
        RollOffMode::Inverse,
        RollOffMode::Linear,
        RollOffMode::LinearSquare,
        RollOffMode::InverseTapered,
    ] {
        let mut last = 1.0;
        for i in 0..2_000 {
            let g = roll_off_gain(mode, i as f32 * 0.6, 10.0, 1_000.0);
            assert!(g <= last + 1e-7, "{mode:?} rises at {i}");
            assert!((0.0..=1.0).contains(&g));
            last = g;
        }
    }
}

#[test]
fn degenerate_inputs_are_safe() {
    // min == max: a hard edge.
    assert_eq!(roll_off_gain(RollOffMode::Linear, 10.0, 10.0, 10.0), 1.0);
    assert_eq!(roll_off_gain(RollOffMode::Linear, 10.01, 10.0, 10.0), 0.0);
    // max below min is raised to min.
    assert_eq!(roll_off_gain(RollOffMode::Inverse, 15.0, 20.0, 5.0), 1.0);
    // Non-finite distance is silent; negative distance counts as zero.
    assert_eq!(
        roll_off_gain(RollOffMode::Inverse, f32::NAN, 10.0, 100.0),
        0.0
    );
    assert_eq!(
        roll_off_gain(RollOffMode::Inverse, f32::INFINITY, 10.0, 100.0),
        0.0
    );
    assert_eq!(roll_off_gain(RollOffMode::Inverse, -5.0, 10.0, 100.0), 1.0);
    // min 0: Inverse gives silence beyond zero distance instead of dividing by zero.
    assert_eq!(roll_off_gain(RollOffMode::Inverse, 1.0, 0.0, 100.0), 0.0);
}

#[test]
fn equal_power_pan_law() {
    let (l, r) = pan_gains(-1.0);
    assert!(close(l, 1.0) && r.abs() < 1e-6);
    let (l, r) = pan_gains(1.0);
    assert!(l.abs() < 1e-6 && close(r, 1.0));
    let (l, r) = pan_gains(0.0);
    assert!(close(l, std::f32::consts::FRAC_1_SQRT_2) && close(r, std::f32::consts::FRAC_1_SQRT_2));
    for i in -20..=20 {
        let (l, r) = pan_gains(i as f32 / 20.0);
        assert!(close(l * l + r * r, 1.0));
    }
    assert_eq!(pan_gains(5.0), pan_gains(1.0));
    assert_eq!(pan_gains(f32::NAN), pan_gains(0.0));
}

#[test]
fn listener_frame_axes() {
    let default = Listener::default(); // looking down -Z, up +Y
    assert!(
        listener_local(&default, Vec3::new(5.0, 0.0, 0.0))
            .abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-5)
    );
    assert!(
        listener_local(&default, Vec3::new(0.0, 0.0, -5.0))
            .abs_diff_eq(Vec3::new(0.0, 0.0, 5.0), 1e-5)
    );
    assert!(
        listener_local(&default, Vec3::new(0.0, 2.0, 0.0))
            .abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5)
    );
    // Facing +X: +Z is on the right.
    let east = Listener {
        forward: Vec3::X,
        ..Listener::default()
    };
    assert!(
        listener_local(&east, Vec3::new(0.0, 0.0, 5.0)).abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-5)
    );
    // Translation and a sloppy, non-normalised basis.
    let sloppy = Listener {
        position: Vec3::new(1.0, 1.0, 1.0),
        forward: Vec3::new(0.0, 0.0, -3.0),
        up: Vec3::new(0.0, 2.0, -0.5),
        velocity: Vec3::ZERO,
    };
    assert!(
        listener_local(&sloppy, Vec3::new(4.0, 1.0, 1.0))
            .abs_diff_eq(Vec3::new(3.0, 0.0, 0.0), 1e-5)
    );
    // Degenerate basis does not produce NaN.
    let broken = Listener {
        forward: Vec3::ZERO,
        up: Vec3::ZERO,
        ..Listener::default()
    };
    assert!(listener_local(&broken, Vec3::new(1.0, 2.0, 3.0)).is_finite());
}

#[test]
fn spatialize_source_on_the_right() {
    let s = spatialize(
        &Listener::default(),
        &Emitter::at(Vec3::new(20.0, 0.0, 0.0)),
        RollOff {
            mode: RollOffMode::Inverse,
            min_distance: 10.0,
            max_distance: 1_000.0,
        },
        &SpatialSettings::default(),
        0.0,
    );
    assert!(close(s.gain, 0.5));
    assert!(s.left.abs() < 1e-6);
    assert!(close(s.right, 0.5));
    assert_eq!(s.doppler, 1.0);
    assert_eq!(s.cutoff_hz, OPEN_CUTOFF_HZ);
}

#[test]
fn rear_sources_are_darker_and_front_sources_are_open() {
    let settings = SpatialSettings::default();
    let roll_off = RollOff::default();
    let front = spatialize(
        &Listener::default(),
        &Emitter::at(Vec3::new(0.0, 0.0, -5.0)),
        roll_off,
        &settings,
        0.0,
    );
    let behind = spatialize(
        &Listener::default(),
        &Emitter::at(Vec3::new(0.0, 0.0, 5.0)),
        roll_off,
        &settings,
        0.0,
    );
    assert!(close(front.cutoff_hz, OPEN_CUTOFF_HZ));
    assert!((behind.cutoff_hz - settings.rear_cutoff_hz).abs() < 1.0);
    // Same loudness and centred either way.
    assert!(close(front.left, behind.left) && close(front.left, front.right));
}

#[test]
fn occlusion_scales_gain_and_cutoff() {
    let settings = SpatialSettings::default();
    let e = Emitter::at(Vec3::new(0.0, 0.0, -5.0));
    let clear = spatialize(&Listener::default(), &e, RollOff::default(), &settings, 0.0);
    let half = spatialize(&Listener::default(), &e, RollOff::default(), &settings, 0.5);
    let full = spatialize(&Listener::default(), &e, RollOff::default(), &settings, 1.0);
    assert!(close(clear.gain, 1.0));
    assert!(close(
        half.gain,
        1.0 - 0.5 * (1.0 - settings.occlusion_gain)
    ));
    assert!(close(full.gain, settings.occlusion_gain));
    assert!((full.cutoff_hz - settings.occlusion_cutoff_hz).abs() < 0.5);
    let geometric = (OPEN_CUTOFF_HZ * settings.occlusion_cutoff_hz).sqrt();
    assert!((half.cutoff_hz - geometric).abs() < 1.0);
    // Out-of-range occlusion is clamped.
    assert_eq!(
        spatialize(&Listener::default(), &e, RollOff::default(), &settings, 7.0),
        full
    );
}

#[test]
fn rolloff_scale_multiplies_distance() {
    let settings = SpatialSettings {
        rolloff_scale: 2.0,
        ..SpatialSettings::default()
    };
    let s = spatialize(
        &Listener::default(),
        &Emitter::at(Vec3::new(20.0, 0.0, 0.0)),
        RollOff {
            mode: RollOffMode::Inverse,
            min_distance: 10.0,
            max_distance: 1_000.0,
        },
        &settings,
        0.0,
    );
    assert!(close(s.gain, 0.25));
}

#[test]
fn doppler_matches_formula() {
    let settings = SpatialSettings::default();
    let c = SPEED_OF_SOUND_MPS * settings.distance_factor;
    let listener = Listener::default();
    // Source 100 studs in front, moving towards the listener at 0.2c.
    let approaching = Emitter {
        position: Vec3::new(0.0, 0.0, -100.0),
        velocity: Vec3::new(0.0, 0.0, 0.2 * c),
    };
    assert!(close(
        doppler_factor(&listener, &approaching, &settings),
        1.0 / 0.8
    ));
    let receding = Emitter {
        velocity: -approaching.velocity,
        ..approaching
    };
    assert!(close(
        doppler_factor(&listener, &receding, &settings),
        1.0 / 1.2
    ));
    // Listener moving towards a still source at 0.1c.
    let still = Emitter::at(Vec3::new(0.0, 0.0, -100.0));
    let moving = Listener {
        velocity: Vec3::new(0.0, 0.0, -0.1 * c),
        ..Listener::default()
    };
    assert!(close(doppler_factor(&moving, &still, &settings), 1.1));
    // Sideways motion has no Doppler.
    let sideways = Emitter {
        velocity: Vec3::new(50.0, 0.0, 0.0),
        ..still
    };
    assert!(close(doppler_factor(&listener, &sideways, &settings), 1.0));
    // DopplerScale 0 disables it; supersonic speeds are clamped.
    let off = SpatialSettings {
        doppler_scale: 0.0,
        ..settings
    };
    assert_eq!(doppler_factor(&listener, &approaching, &off), 1.0);
    let supersonic = Emitter {
        velocity: Vec3::new(0.0, 0.0, 10.0 * c),
        ..still
    };
    assert_eq!(
        doppler_factor(&listener, &supersonic, &settings),
        DOPPLER_RANGE.1
    );
}
