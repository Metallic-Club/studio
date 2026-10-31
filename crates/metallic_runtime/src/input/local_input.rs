//! Presentation-bound conversion from evaluated actions to canonical command data.

#[cfg(test)]
mod tests {
    use super::super::{
        Control, Device, FrameInput, InputConfig, InputRuntime, RawEvent, default_character_controls,
    };
    use crate::LocalInput;
    use std::f32::consts::FRAC_PI_2;

    fn evaluated(events: &[RawEvent]) -> FrameInput {
        let config = InputConfig {
            contexts: vec![default_character_controls()],
            warnings: Vec::new(),
        };
        let mut runtime = InputRuntime::new(&config);
        for event in events {
            runtime.push(*event);
        }
        runtime.evaluate(1.0, 1.0 / 60.0);
        runtime.frame_input()
    }

    #[test]
    fn frame_adapter_preserves_evaluated_keyboard_composites_and_flags() {
        let keyboard = Device::KeyboardMouse;
        let frame = evaluated(&[
            RawEvent::button(keyboard, Control::W, 1.0, 1.0),
            RawEvent::button(keyboard, Control::D, 1.0, 1.0),
            RawEvent::button(keyboard, Control::Space, 1.0, 1.0),
            RawEvent::button(keyboard, Control::LeftShift, 1.0, 1.0),
        ]);
        // Composites have already been normalized by InputRuntime.
        assert!((frame.movement[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((frame.movement[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let command = LocalInput::from_frame(&frame, FRAC_PI_2);
        assert_eq!((command.move_x, command.move_z), (90, 90));
        assert_eq!(command.yaw, 16_384);
        assert!(command.jump && command.run);
        assert_eq!(
            command,
            LocalInput::from_components(frame.movement, frame.jump, frame.run, FRAC_PI_2)
        );
    }

    #[test]
    fn frame_adapter_does_not_apply_a_second_analog_deadzone() {
        for (raw, expected) in [(0.17, 0), (0.2, 3), (0.6, 65), (1.0, 127)] {
            let frame = evaluated(&[RawEvent::vector(
                Device::Gamepad(0),
                Control::Thumbstick1,
                [raw, 0.0],
                1.0,
            )]);
            if raw == 0.2 {
                assert!(frame.movement[0] > 0.0 && frame.movement[0] < 0.18);
            }
            let command = LocalInput::from_frame(&frame, 0.0);
            assert_eq!(command.move_x, expected, "raw {raw}, frame {frame:?}");
            assert_eq!(command.move_z, 0);
        }
        // Small values may already have been processed by a custom action binding.
        let frame = FrameInput {
            movement: [0.01, -0.01],
            ..Default::default()
        };
        let command = LocalInput::from_frame(&frame, 0.0);
        assert_eq!((command.move_x, command.move_z), (1, -1));
    }

    #[test]
    fn frame_adapter_uses_absolute_camera_yaw_and_ignores_orbit_and_zoom() {
        let mut frame = FrameInput {
            movement: [0.5, -0.25],
            jump: true,
            run: false,
            orbit: [123.0, -456.0],
            zoom: 10.0,
        };
        let before = frame;
        let command = LocalInput::from_frame(&frame, FRAC_PI_2);
        assert_eq!(frame, before);
        assert_eq!((command.move_x, command.move_z, command.yaw), (64, -32, 16_384));
        frame.orbit = [f32::NAN, f32::INFINITY];
        frame.zoom = f32::NEG_INFINITY;
        assert_eq!(LocalInput::from_frame(&frame, FRAC_PI_2), command);
        assert_eq!(LocalInput::from_frame(&frame, 0.0).yaw, 0);
    }

    #[test]
    fn frame_adapter_sanitizes_only_non_finite_command_components() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for (movement, expected) in [([invalid, 0.5], (0, 64)), ([0.5, invalid], (64, 0))] {
                let frame = FrameInput {
                    movement,
                    jump: false,
                    run: true,
                    ..Default::default()
                };
                let command = LocalInput::from_frame(&frame, invalid);
                assert_eq!((command.move_x, command.move_z), expected);
                assert_eq!(command.yaw, 0);
                assert!(!command.jump && command.run);
                assert!(command.is_canonical());
            }
        }
    }
}
