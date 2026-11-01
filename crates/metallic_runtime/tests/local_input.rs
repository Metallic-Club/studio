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

#[test]
fn every_canonical_axis_code_round_trips_without_losing_either_axis() {
    for axis in -127i8..=127 {
        let value = f32::from(axis) / 127.0;
        for (movement, expected) in [([value, 0.0], (axis, 0)), ([0.0, value], (0, axis))] {
            let command = LocalInput::from_components(movement, true, false, 0.0);
            assert_eq!((command.move_x, command.move_z), expected, "{axis}");
            assert!(command.is_canonical());
            assert!(command.jump && !command.run);
        }
    }
}

#[test]
fn every_yaw_code_round_trips_through_its_decoded_angle() {
    let step = TAU / 65_536.0;
    for yaw in u16::MIN..=u16::MAX {
        let angle = f32::from(yaw) * step;
        let command = LocalInput::from_components([0.0; 2], false, true, angle);
        assert_eq!(command.yaw, yaw);
        assert_eq!((command.move_x, command.move_z), (0, 0));
        assert!(!command.jump && command.run);
    }
}

#[test]
fn exact_axis_ties_and_neighboring_floats_round_away_from_zero() {
    // +/-0.5 * 127 = +/-63.5 exactly in f32, so these are genuine half-step ties.
    for (value, expected) in [
        (0.5f32.next_down(), 63),
        (0.5, 64),
        (0.5f32.next_up(), 64),
        ((-0.5f32).next_up(), -63),
        (-0.5, -64),
        ((-0.5f32).next_down(), -64),
    ] {
        let command = LocalInput::from_components([value, -value], false, false, 0.0);
        assert_eq!((command.move_x, command.move_z), (expected, -expected));
    }
}

#[test]
fn yaw_half_step_neighbors_and_turn_seam_have_stable_codes() {
    let half_step = TAU / 131_072.0;
    for (angle, expected) in [
        (half_step.next_down(), 0),
        (half_step, 1),
        (half_step.next_up(), 1),
        (TAU - 3.0 * half_step / 2.0, 65_535),
        (TAU - half_step / 2.0, 0),
        (TAU, 0),
    ] {
        assert_eq!(
            LocalInput::from_components([0.0; 2], false, false, angle).yaw,
            expected,
            "angle {angle:?}"
        );
    }
}

#[test]
fn non_finite_second_axis_does_not_erase_the_first_or_held_flags() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let command = LocalInput::from_components([0.5, invalid], true, true, FRAC_PI_2);
        assert_eq!(
            (command.move_x, command.move_z, command.yaw),
            (64, 0, 16_384)
        );
        assert!(command.jump && command.run);
    }
}

#[test]
fn seeded_float_bit_patterns_always_produce_finite_bounded_commands() {
    fn next(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }
    let mut seed = 0x349c_1a7eu32;
    for _ in 0..10_000 {
        let x = f32::from_bits(next(&mut seed));
        let z = f32::from_bits(next(&mut seed));
        let yaw = f32::from_bits(next(&mut seed));
        let jump = next(&mut seed) & 1 != 0;
        let run = next(&mut seed) & 1 != 0;
        let command = LocalInput::from_components([x, z], jump, run, yaw);
        assert!(command.is_canonical());
        if !x.is_finite() {
            assert_eq!(command.move_x, 0);
        }
        if !z.is_finite() {
            assert_eq!(command.move_z, 0);
        }
        if !yaw.is_finite() {
            assert_eq!(command.yaw, 0);
        }
        let decoded = command.to_move_input();
        assert!(decoded.dir.x.is_finite() && decoded.dir.z.is_finite());
        assert_eq!(decoded.dir.y, 0.0);
        assert!(length(decoded.dir) <= 1.0 + 1e-12);
        assert_eq!((decoded.jump, decoded.run), (jump, run));
    }
}
