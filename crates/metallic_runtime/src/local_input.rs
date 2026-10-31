//! Canonical movement data shared by future local and remote simulation drivers.
use metallic_core::Vec3;
use metallic_physics::MoveInput;

const YAW_STEPS: f32 = 65_536.0;

/// Quantized camera-relative movement, held buttons and view yaw.
///
/// This is command data, not a serialized packet or an authenticated player identity.
/// Its Rust memory layout is not a wire format. Protocol decoders must reject
/// noncanonical axes with [`Self::is_canonical`] before admitting a command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalInput {
    /// Rightward movement, -127 through 127; -128 is noncanonical.
    pub move_x: i8,
    /// Forward movement, -127 through 127; -128 is noncanonical.
    pub move_z: i8,
    /// Whether jump is held.
    pub jump: bool,
    /// Whether run is held.
    pub run: bool,
    /// One turn divided into 65,536 steps; zero faces world -Z.
    pub yaw: u16,
}

impl LocalInput {
    /// Quantizes finite local input, rounding axis ties away from zero.
    ///
    /// Each non-finite component independently becomes zero. Finite axes saturate
    /// to [-1, 1]; finite yaw in radians wraps through one turn before rounding. This does not
    /// alter the current Game input path or provide network admission.
    pub fn from_components(movement: [f32; 2], jump: bool, run: bool, yaw: f32) -> Self {
        fn axis(value: f32) -> i8 {
            if value.is_finite() {
                (value.clamp(-1.0, 1.0) * 127.0).round() as i8
            } else {
                0
            }
        }
        let yaw = if yaw.is_finite() {
            // Rounding can reach 65,536 at the seam; the final cast wraps it to zero.
            (yaw.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * YAW_STEPS).round()
                as u32 as u16
        } else {
            0
        };
        Self {
            move_x: axis(movement[0]),
            move_z: axis(movement[1]),
            jump,
            run,
            yaw,
        }
    }

    /// Whether the axes are outputs of the canonical local quantizer.
    pub const fn is_canonical(self) -> bool {
        self.move_x != i8::MIN && self.move_z != i8::MIN
    }

    /// Decodes to finite world-space movement with horizontal length at most one.
    ///
    /// Half-stick magnitude is retained. Diagonals are shortened, and even a
    /// manually constructed -128 axis is saturated defensively. This fallback
    /// does not replace a protocol decoder's canonical-value check.
    pub fn to_move_input(self) -> MoveInput {
        let movement = [
            f32::from(self.move_x.max(-127)) / 127.0,
            f32::from(self.move_z.max(-127)) / 127.0,
        ];
        let yaw = f32::from(self.yaw) / YAW_STEPS * std::f32::consts::TAU;
        let mut dir = move_direction(movement, yaw);
        let length = (dir.x * dir.x + dir.z * dir.z).sqrt();
        if length > 1.0 {
            dir = Vec3::new(dir.x / length, 0.0, dir.z / length);
        }
        MoveInput {
            dir,
            jump: self.jump,
            run: self.run,
        }
    }
}

/// Converts camera-relative wishes (right, forward) to a flat world-space vector.
///
/// Yaw is in radians; zero faces -Z and positive yaw turns right. This preserves the existing
/// CameraController transform, including its caller-controlled magnitude; use
/// [`LocalInput::to_move_input`] for bounded quantized commands. Inputs are trusted
/// finite values. This function does not promise cross-platform physics determinism.
pub fn move_direction(movement: [f32; 2], yaw: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let forward = (f64::from(sy), f64::from(-cy));
    let right = (f64::from(cy), f64::from(sy));
    let (mx, my) = (f64::from(movement[0]), f64::from(movement[1]));
    Vec3::new(
        right.0 * mx + forward.0 * my,
        0.0,
        right.1 * mx + forward.1 * my,
    )
}
