//! Snapshot quantization (spec 4.5): positions in 64-stud cells at 1/64 stud, orientations as smallest-three
//! quaternions, velocities at 1/8 stud/s.
//!
//! Every quantizer accepts any `f64`: NaN becomes 0 and out-of-range values (infinities included) clamp, so nothing
//! panics and the output always encodes.

use metallic_core::{CFrame, Vec3};

/// Fixed-point steps per stud for positions.
const POS_SCALE: f64 = 64.0;
/// Steps per 64-stud cell (12 bits).
const CELL_STEPS: i64 = 4096;
/// Bits of the signed fixed-point Y.
const Y_BITS: u32 = 20;
/// Fixed-point steps per stud/s for velocities.
const VEL_SCALE: f64 = 8.0;
/// Largest velocity step (12-bit signed, symmetric): 2047 / 8 = 255.875 studs/s.
const VEL_MAX: i16 = 2047;
/// Bits per smallest-three component.
const ROT_BITS: u32 = 10;
/// Largest value of a smallest-three component code.
const ROT_MAX: f64 = ((1u32 << ROT_BITS) - 1) as f64;

/// A quantized position: the 64-stud grid cell, the 1/64-stud offset inside it on X and Z, and Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantPos {
    /// Grid cell on X (`floor(x / 64)`, after rounding to 1/64 stud).
    pub cell_x: i32,
    /// Grid cell on Z.
    pub cell_z: i32,
    /// X offset inside the cell in 1/64 studs, 0..4096 (12 bits).
    pub x: u16,
    /// Z offset inside the cell in 1/64 studs, 0..4096 (12 bits).
    pub z: u16,
    /// Height.
    pub y: QuantY,
}

/// A quantized height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuantY {
    /// 1/64 stud fixed point, 20-bit signed: heights within -8,192..8,192 studs.
    Fixed(i32),
    /// A height outside the fixed range, as a finite `f32`.
    Raw(f32),
}

/// Rounds `v` studs to whole 1/64 steps, clamped to `[lo, hi]`; NaN gives 0.
fn steps(v: f64, lo: i64, hi: i64) -> i64 {
    if v.is_nan() {
        return 0;
    }
    // `as` saturates for out-of-range floats; the clamp then bounds it exactly.
    ((v * POS_SCALE).round() as i64).clamp(lo, hi)
}

fn split_axis(v: f64) -> (i32, u16) {
    let lo = i64::from(i32::MIN) * CELL_STEPS;
    let hi = i64::from(i32::MAX) * CELL_STEPS + (CELL_STEPS - 1);
    let n = steps(v, lo, hi);
    // In range by the clamp above: the cell fits i32 and the offset 0..4096.
    (
        n.div_euclid(CELL_STEPS) as i32,
        n.rem_euclid(CELL_STEPS) as u16,
    )
}

/// Quantizes a position: rounds to the nearest 1/64 stud (error at most 1/128 stud).
pub fn quantize_position(p: Vec3) -> QuantPos {
    let (cell_x, x) = split_axis(p.x);
    let (cell_z, z) = split_axis(p.z);
    let y_lo = -(1i64 << (Y_BITS - 1));
    let y_hi = (1i64 << (Y_BITS - 1)) - 1;
    let y = if p.y.is_nan() {
        QuantY::Fixed(0)
    } else {
        let n = (p.y * POS_SCALE).round();
        if n >= y_lo as f64 && n <= y_hi as f64 {
            QuantY::Fixed(n as i32)
        } else {
            QuantY::Raw(p.y.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32)
        }
    };
    QuantPos {
        cell_x,
        cell_z,
        x,
        z,
        y,
    }
}

/// The position a [`QuantPos`] stands for.
pub fn dequantize_position(q: &QuantPos) -> Vec3 {
    let axis = |cell: i32, offset: u16| {
        (i64::from(cell) * CELL_STEPS + i64::from(offset)) as f64 / POS_SCALE
    };
    let y = match q.y {
        QuantY::Fixed(n) => f64::from(n) / POS_SCALE,
        QuantY::Raw(f) if f.is_finite() => f64::from(f),
        QuantY::Raw(_) => 0.0,
    };
    Vec3::new(axis(q.cell_x, q.x), y, axis(q.cell_z, q.z))
}

/// Quantizes an `Orientation` (degrees, applied Y then X then Z) as smallest-three: the index of the largest
/// quaternion component in the top 2 bits, then the other three in 10 bits each. Each kept component is rounded down
/// or up, and of the 8 combinations the encoder keeps the one whose decoded rotation is closest to the input: plain
/// rounding reaches about 0.27 degrees near `[0.5, 0.5, 0.5, 0.5]` (the rebuilt fourth component adds the three
/// rounding errors), the best combination stays under about 0.16 degrees. Non-finite angles count as 0.
pub fn quantize_orientation(degrees: Vec3) -> u32 {
    let clean = |a: f64| if a.is_finite() { a } else { 0.0 };
    let o = Vec3::new(clean(degrees.x), clean(degrees.y), clean(degrees.z));
    let mut q = CFrame::from_position_orientation(Vec3::default(), o).quaternion();
    if q.iter().any(|c| !c.is_finite()) {
        q = [0.0, 0.0, 0.0, 1.0];
    }
    let mut largest = 0;
    for i in 1..4 {
        if q[i].abs() > q[largest].abs() {
            largest = i;
        }
    }
    // q and -q are the same rotation: make the dropped component positive so it can be rebuilt as a square root.
    let sign = if q[largest] < 0.0 { -1.0 } else { 1.0 };
    // Floor of each kept component's code; the components lie in [-1/sqrt 2, 1/sqrt 2].
    let mut floors = [0u32; 3];
    for (slot, c) in floors.iter_mut().zip(
        q.iter()
            .enumerate()
            .filter(|(i, _)| *i != largest)
            .map(|(_, c)| *c),
    ) {
        let unit = (c * sign * std::f64::consts::SQRT_2 + 1.0) * 0.5;
        *slot = (unit.clamp(0.0, 1.0) * ROT_MAX).floor() as u32;
    }
    let top = ROT_MAX as u32;
    let mut best = (f64::NEG_INFINITY, 0u32);
    for combo in 0..8u32 {
        let mut code = (largest as u32) << (3 * ROT_BITS);
        for (k, floor) in floors.iter().enumerate() {
            let n = (floor + ((combo >> k) & 1)).min(top);
            code |= n << (ROT_BITS * (2 - k as u32));
        }
        // |q . decoded| grows as the angle between the two rotations shrinks.
        let d = decode_quaternion(code);
        let dot = (q[0] * d[0] + q[1] * d[1] + q[2] * d[2] + q[3] * d[3]).abs();
        if dot > best.0 {
            best = (dot, code);
        }
    }
    best.1
}

/// The unit quaternion `[x, y, z, w]` a smallest-three code stands for.
fn decode_quaternion(code: u32) -> [f64; 4] {
    let largest = (code >> (3 * ROT_BITS)) as usize;
    let mut q = [0.0f64; 4];
    let mut shift = 2 * ROT_BITS;
    let mut sum = 0.0;
    for (i, slot) in q.iter_mut().enumerate() {
        if i == largest {
            continue;
        }
        let n = f64::from((code >> shift) & ((1 << ROT_BITS) - 1));
        let c = (n / ROT_MAX * 2.0 - 1.0) / std::f64::consts::SQRT_2;
        *slot = c;
        sum += c * c;
        shift = shift.saturating_sub(ROT_BITS);
    }
    // A hostile code can make the three exceed length 1: the clamp keeps the root real, then normalise.
    if let Some(slot) = q.get_mut(largest) {
        *slot = (1.0 - sum).max(0.0).sqrt();
    }
    let n = q.iter().map(|c| c * c).sum::<f64>().sqrt();
    q.map(|c| c / n)
}

/// The `Orientation` (degrees) a [`quantize_orientation`] code stands for. Any 32-bit code gives finite angles.
pub fn dequantize_orientation(code: u32) -> Vec3 {
    CFrame::from_quaternion(Vec3::default(), decode_quaternion(code)).orientation_degrees()
}

/// Quantizes a velocity to 1/8 stud/s per axis, clamped to +-255.875 (12-bit signed). NaN gives 0.
pub fn quantize_velocity(v: Vec3) -> [i16; 3] {
    let q = |c: f64| {
        if c.is_nan() {
            0
        } else {
            ((c * VEL_SCALE)
                .round()
                .clamp(-f64::from(VEL_MAX), f64::from(VEL_MAX))) as i16
        }
    };
    [q(v.x), q(v.y), q(v.z)]
}

/// The velocity (studs/s) a [`quantize_velocity`] result stands for.
pub fn dequantize_velocity(q: [i16; 3]) -> Vec3 {
    Vec3::new(
        f64::from(q[0]) / VEL_SCALE,
        f64::from(q[1]) / VEL_SCALE,
        f64::from(q[2]) / VEL_SCALE,
    )
}
