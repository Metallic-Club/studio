//! Every easing style and direction against reference values computed independently from the
//! standard (easings.net / Penner) formulas, written per direction.
#![allow(clippy::approx_constant)] // reference values are literal on purpose

use metallic_anim::{EasingDirection, EasingStyle, ease};

const TS: [f32; 5] = [0.1, 0.25, 0.5, 0.75, 0.9];

#[rustfmt::skip]
const TABLE: &[(EasingStyle, EasingDirection, [f32; 5])] = &[
    (EasingStyle::Linear, EasingDirection::In, [0.1, 0.25, 0.5, 0.75, 0.9]),
    (EasingStyle::Linear, EasingDirection::Out, [0.1, 0.25, 0.5, 0.75, 0.9]),
    (EasingStyle::Linear, EasingDirection::InOut, [0.1, 0.25, 0.5, 0.75, 0.9]),
    (EasingStyle::Sine, EasingDirection::In, [0.0123117, 0.0761205, 0.292893, 0.617317, 0.843566]),
    (EasingStyle::Sine, EasingDirection::Out, [0.156434, 0.382683, 0.707107, 0.92388, 0.987688]),
    (EasingStyle::Sine, EasingDirection::InOut, [0.0244717, 0.146447, 0.5, 0.853553, 0.975528]),
    (EasingStyle::Back, EasingDirection::In, [-0.0143142, -0.0641366, -0.0876975, 0.18259, 0.591172]),
    (EasingStyle::Back, EasingDirection::Out, [0.408828, 0.81741, 1.0877, 1.06414, 1.01431]),
    (EasingStyle::Back, EasingDirection::InOut, [-0.0232253, -0.0438488, 0.5, 1.04385, 1.02323]),
    (EasingStyle::Quad, EasingDirection::In, [0.01, 0.0625, 0.25, 0.5625, 0.81]),
    (EasingStyle::Quad, EasingDirection::Out, [0.19, 0.4375, 0.75, 0.9375, 0.99]),
    (EasingStyle::Quad, EasingDirection::InOut, [0.02, 0.125, 0.5, 0.875, 0.98]),
    (EasingStyle::Quart, EasingDirection::In, [0.0001, 0.00390625, 0.0625, 0.316406, 0.6561]),
    (EasingStyle::Quart, EasingDirection::Out, [0.3439, 0.683594, 0.9375, 0.996094, 0.9999]),
    (EasingStyle::Quart, EasingDirection::InOut, [0.0008, 0.03125, 0.5, 0.96875, 0.9992]),
    (EasingStyle::Quint, EasingDirection::In, [1e-05, 0.000976562, 0.03125, 0.237305, 0.59049]),
    (EasingStyle::Quint, EasingDirection::Out, [0.40951, 0.762695, 0.96875, 0.999023, 0.99999]),
    (EasingStyle::Quint, EasingDirection::InOut, [0.00016, 0.015625, 0.5, 0.984375, 0.99984]),
    (EasingStyle::Bounce, EasingDirection::In, [0.011875, 0.0273438, 0.234375, 0.527344, 0.924375]),
    (EasingStyle::Bounce, EasingDirection::Out, [0.075625, 0.472656, 0.765625, 0.972656, 0.988125]),
    (EasingStyle::Bounce, EasingDirection::InOut, [0.03, 0.117188, 0.5, 0.882812, 0.97]),
    (EasingStyle::Elastic, EasingDirection::In, [0.00195312, -0.00552427, -0.015625, 0.0883883, -0.25]),
    (EasingStyle::Elastic, EasingDirection::Out, [1.25, 0.911612, 1.01562, 1.00552, 0.998047]),
    (EasingStyle::Elastic, EasingDirection::InOut, [-0.000976563, -0.0078125, 0.5, 1.00781, 1.00098]),
    (EasingStyle::Exponential, EasingDirection::In, [0.00195312, 0.00552427, 0.03125, 0.176777, 0.5]),
    (EasingStyle::Exponential, EasingDirection::Out, [0.5, 0.823223, 0.96875, 0.994476, 0.998047]),
    (EasingStyle::Exponential, EasingDirection::InOut, [0.00195312, 0.015625, 0.5, 0.984375, 0.998047]),
    (EasingStyle::Circular, EasingDirection::In, [0.00501256, 0.0317542, 0.133975, 0.338562, 0.56411]),
    (EasingStyle::Circular, EasingDirection::Out, [0.43589, 0.661438, 0.866025, 0.968246, 0.994987]),
    (EasingStyle::Circular, EasingDirection::InOut, [0.0101021, 0.0669873, 0.5, 0.933013, 0.989898]),
    (EasingStyle::Cubic, EasingDirection::In, [0.001, 0.015625, 0.125, 0.421875, 0.729]),
    (EasingStyle::Cubic, EasingDirection::Out, [0.271, 0.578125, 0.875, 0.984375, 0.999]),
    (EasingStyle::Cubic, EasingDirection::InOut, [0.004, 0.0625, 0.5, 0.9375, 0.996]),
];

#[test]
fn every_style_and_direction_matches_reference_values() {
    assert_eq!(
        TABLE.len(),
        EasingStyle::ALL.len() * EasingDirection::ALL.len()
    );
    for (style, dir, expected) in TABLE {
        for (t, want) in TS.iter().zip(expected) {
            let got = ease(*style, *dir, *t);
            assert!(
                (got - want).abs() < 2e-5,
                "{style:?} {dir:?} at {t}: got {got}, want {want}"
            );
        }
    }
}

#[test]
fn endpoints_are_exact_and_input_is_clamped() {
    for style in EasingStyle::ALL {
        for dir in EasingDirection::ALL {
            assert_eq!(ease(style, dir, 0.0), 0.0, "{style:?} {dir:?}");
            assert_eq!(ease(style, dir, 1.0), 1.0, "{style:?} {dir:?}");
            assert_eq!(ease(style, dir, -3.0), 0.0);
            assert_eq!(ease(style, dir, 7.0), 1.0);
            assert_eq!(ease(style, dir, f32::NAN), 0.0);
        }
    }
}

#[test]
fn out_mirrors_in() {
    for style in EasingStyle::ALL {
        for i in 1..20 {
            let t = i as f32 / 20.0;
            let a = ease(style, EasingDirection::Out, t);
            let b = 1.0 - ease(style, EasingDirection::In, 1.0 - t);
            assert!((a - b).abs() < 1e-5, "{style:?} at {t}");
        }
    }
}

#[test]
fn only_back_and_elastic_overshoot() {
    for style in EasingStyle::ALL {
        let mut leaves = false;
        for dir in EasingDirection::ALL {
            for i in 0..=200 {
                let v = ease(style, dir, i as f32 / 200.0);
                leaves |= !(-1e-6..=1.0 + 1e-6).contains(&v);
            }
        }
        assert_eq!(
            leaves,
            matches!(style, EasingStyle::Back | EasingStyle::Elastic),
            "{style:?}"
        );
    }
}

#[test]
fn defaults_match_roblox() {
    assert_eq!(EasingStyle::default(), EasingStyle::Quad);
    assert_eq!(EasingDirection::default(), EasingDirection::Out);
}
