use metallic_core::Vec3;
use metallic_runtime::{LocalInput, move_direction};
use std::f32::consts::{FRAC_PI_2, TAU};

fn length(dir: Vec3) -> f64 {
    dir.x.hypot(dir.z)
}

#[test]
fn signed_rounding_saturation_and_flags_are_canonical() {
    for (value, expected) in [
        (-2.0, -127),
        (-1.0, -127),
        (-0.75, -95),
        (-0.5, -64),
        (-0.25, -32),
        (0.0, 0),
        (0.25, 32),
        (0.5, 64),
        (0.75, 95),
        (1.0, 127),
        (2.0, 127),
    ] {
        let input = LocalInput::from_components([value, -value], true, false, 0.0);
        assert_eq!((input.move_x, input.move_z), (expected, -expected));
        assert!(input.is_canonical());
        let decoded = input.to_move_input();
        assert!(decoded.jump);
        assert!(!decoded.run);
    }
    assert_eq!(LocalInput::default().to_move_input(), Default::default());
}

#[test]
fn yaw_wraps_and_rounds_at_the_turn_seam() {
    for (yaw, expected) in [
        (0.0, 0),
        (FRAC_PI_2, 16_384),
        (TAU / 2.0, 32_768),
        (-FRAC_PI_2, 49_152),
        (TAU, 0),
        (-TAU, 0),
        (-1e-12, 0),
    ] {
        assert_eq!(
            LocalInput::from_components([0.0; 2], false, false, yaw).yaw,
            expected
        );
    }
    let near_seam = TAU - TAU / 65_536.0 * 0.25;
    assert_eq!(
        LocalInput::from_components([0.0; 2], false, false, near_seam).yaw,
        0
    );
    for yaw in [f32::MIN, f32::MAX] {
        let input = LocalInput::from_components([0.5, 0.0], false, true, yaw);
        assert!(input.is_canonical());
        assert!(length(input.to_move_input().dir).is_finite());
    }
}

#[test]
fn non_finite_local_components_independently_become_zero() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let input = LocalInput::from_components([invalid, 0.5], false, true, invalid);
        assert_eq!((input.move_x, input.move_z, input.yaw), (0, 64, 0));
        let decoded = input.to_move_input();
        assert!(decoded.run);
        assert!(!decoded.jump);
        assert_eq!(decoded.dir.x, 0.0);
        assert!((decoded.dir.z + 64.0 / 127.0).abs() < 1e-7);
    }
}

#[test]
fn rotation_diagonal_speed_and_half_stick_magnitude_are_preserved() {
    let right = LocalInput {
        move_z: 127,
        yaw: 16_384,
        ..Default::default()
    }
    .to_move_input();
    assert!((right.dir.x - 1.0).abs() < 1e-6);
    assert!(right.dir.z.abs() < 1e-6);
    let diagonal = LocalInput {
        move_x: 127,
        move_z: 127,
        ..Default::default()
    }
    .to_move_input();
    assert!((length(diagonal.dir) - 1.0).abs() < 1e-12);
    assert!((diagonal.dir.x + diagonal.dir.z).abs() < 1e-12);
    for yaw in [0, 1, 16_384, 40_000, 65_535] {
        let half = LocalInput {
            move_z: 64,
            yaw,
            ..Default::default()
        }
        .to_move_input();
        assert!((length(half.dir) - 64.0 / 127.0).abs() < 1e-6);
    }
    // The shared legacy transform deliberately retains caller magnitude.
    assert_eq!(move_direction([0.5, 1.0], 0.0), Vec3::new(0.5, 0.0, -1.0));
}

#[test]
fn every_axis_byte_pair_decodes_to_finite_bounded_movement() {
    for x in i8::MIN..=i8::MAX {
        for z in i8::MIN..=i8::MAX {
            for yaw in [0, 1, 16_384, 32_768, 65_535] {
                let input = LocalInput {
                    move_x: x,
                    move_z: z,
                    yaw,
                    jump: x & 1 == 0,
                    run: z & 1 == 0,
                };
                let decoded = input.to_move_input();
                assert!(decoded.dir.x.is_finite() && decoded.dir.z.is_finite());
                assert_eq!(decoded.dir.y, 0.0);
                assert!(length(decoded.dir) <= 1.0 + 1e-12, "{input:?}");
                assert_eq!((decoded.jump, decoded.run), (input.jump, input.run));
            }
        }
    }
    for (x, z) in [(i8::MIN, 0), (0, i8::MIN), (i8::MIN, i8::MIN)] {
        assert!(
            !LocalInput {
                move_x: x,
                move_z: z,
                ..Default::default()
            }
            .is_canonical()
        );
    }
}
