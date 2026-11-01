//! A scalar view of every channel type, so the curve editor can draw and edit one number at a
//! time: `f32` is one curve, `Vec3` three, rotations are shown as Euler angles in degrees.

use metallic_anim::glam::{EulerRot, Quat, Vec2, Vec3};
use metallic_anim::{Animatable, Channel, Color3, Curve, Tangent, TangentMode, with_curve};

/// Which handle of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleSide {
    /// The incoming (left) handle.
    In,
    /// The outgoing (right) handle.
    Out,
}

/// Per-type access to scalar components.
pub trait ScalarView: Animatable {
    /// Component labels ("X", "Y", "Z"...).
    const LABELS: &'static [&'static str];
    /// True when the curve editor can show Bezier handles for this type.
    const HANDLES: bool;
    /// Component `c` of a value, in display units.
    fn get(v: &Self, c: usize) -> f32;
    /// Sets component `c` of a value, in display units.
    fn set(v: &mut Self, c: usize, x: f32);
    /// Component `c` of a handle delta (only called when [`Self::HANDLES`] is true).
    fn get_delta(v: &Self, c: usize) -> f32 {
        Self::get(v, c)
    }
    /// Sets component `c` of a handle delta.
    fn set_delta(v: &mut Self, c: usize, x: f32) {
        Self::set(v, c, x);
    }
}

impl ScalarView for f32 {
    const LABELS: &'static [&'static str] = &["Value"];
    const HANDLES: bool = true;
    fn get(v: &Self, _c: usize) -> f32 {
        *v
    }
    fn set(v: &mut Self, _c: usize, x: f32) {
        *v = x;
    }
}

impl ScalarView for Vec2 {
    const LABELS: &'static [&'static str] = &["X", "Y"];
    const HANDLES: bool = true;
    fn get(v: &Self, c: usize) -> f32 {
        v[c.min(1)]
    }
    fn set(v: &mut Self, c: usize, x: f32) {
        v[c.min(1)] = x;
    }
}

impl ScalarView for Vec3 {
    const LABELS: &'static [&'static str] = &["X", "Y", "Z"];
    const HANDLES: bool = true;
    fn get(v: &Self, c: usize) -> f32 {
        v[c.min(2)]
    }
    fn set(v: &mut Self, c: usize, x: f32) {
        v[c.min(2)] = x;
    }
}

impl ScalarView for Color3 {
    const LABELS: &'static [&'static str] = &["R", "G", "B"];
    const HANDLES: bool = true;
    fn get(v: &Self, c: usize) -> f32 {
        match c {
            0 => v.r,
            1 => v.g,
            _ => v.b,
        }
    }
    fn set(v: &mut Self, c: usize, x: f32) {
        match c {
            0 => v.r = x,
            1 => v.g = x,
            _ => v.b = x,
        }
    }
}

impl ScalarView for Quat {
    const LABELS: &'static [&'static str] = &["Pitch X", "Yaw Y", "Roll Z"];
    const HANDLES: bool = false;
    fn get(v: &Self, c: usize) -> f32 {
        let (x, y, z) = v.to_euler(EulerRot::XYZ);
        [x, y, z][c.min(2)].to_degrees()
    }
    fn set(v: &mut Self, c: usize, x: f32) {
        let (a, b, cc) = v.to_euler(EulerRot::XYZ);
        let mut e = [a, b, cc];
        e[c.min(2)] = x.to_radians();
        *v = Quat::from_euler(EulerRot::XYZ, e[0], e[1], e[2]);
    }
}

impl ScalarView for bool {
    const LABELS: &'static [&'static str] = &["On"];
    const HANDLES: bool = false;
    fn get(v: &Self, _c: usize) -> f32 {
        if *v { 1.0 } else { 0.0 }
    }
    fn set(v: &mut Self, _c: usize, x: f32) {
        *v = x >= 0.5;
    }
}

/// Component labels of a channel.
pub fn labels(channel: &Channel) -> &'static [&'static str] {
    fn l<T: ScalarView>(_: &Curve<T>) -> &'static [&'static str] {
        T::LABELS
    }
    with_curve!(channel, c => l(c))
}

/// True when the channel's keys have editable Bezier handles.
pub fn has_handles(channel: &Channel) -> bool {
    fn h<T: ScalarView>(_: &Curve<T>) -> bool {
        T::HANDLES
    }
    with_curve!(channel, c => h(c))
}

/// Component `comp` of the channel's value at `time`.
pub fn sample(channel: &Channel, comp: usize, time: f32) -> Option<f32> {
    fn s<T: ScalarView>(c: &Curve<T>, comp: usize, time: f32) -> Option<f32> {
        c.sample(time).map(|v| T::get(&v, comp))
    }
    with_curve!(channel, c => s(c, comp, time))
}

/// Component `comp` of key `key`'s value.
pub fn key_value(channel: &Channel, key: usize, comp: usize) -> Option<f32> {
    fn k<T: ScalarView>(c: &Curve<T>, key: usize, comp: usize) -> Option<f32> {
        c.keys().get(key).map(|k| T::get(&k.value, comp))
    }
    with_curve!(channel, c => k(c, key, comp))
}

/// Sets component `comp` of key `key`'s value and recomputes handles.
pub fn set_key_value(channel: &mut Channel, key: usize, comp: usize, value: f32) {
    fn k<T: ScalarView>(c: &mut Curve<T>, key: usize, comp: usize, value: f32) {
        if let Some(k) = c.key_mut(key) {
            T::set(&mut k.value, comp, value);
        }
        c.recompute_tangents();
    }
    with_curve!(channel, c => k(c, key, comp, value));
}

/// A key's handle as an absolute `(time, value)` point for component `comp`, or `None` for types
/// without handles.
pub fn handle_point(
    channel: &Channel,
    key: usize,
    comp: usize,
    side: HandleSide,
) -> Option<(f32, f32)> {
    fn h<T: ScalarView>(
        c: &Curve<T>,
        key: usize,
        comp: usize,
        side: HandleSide,
    ) -> Option<(f32, f32)> {
        if !T::HANDLES {
            return None;
        }
        let k = c.keys().get(key)?;
        let v = T::get(&k.value, comp);
        Some(match side {
            HandleSide::In => (
                k.time - k.in_tangent.time,
                v - T::get_delta(&k.in_tangent.value, comp),
            ),
            HandleSide::Out => (
                k.time + k.out_tangent.time,
                v + T::get_delta(&k.out_tangent.value, comp),
            ),
        })
    }
    with_curve!(channel, c => h(c, key, comp, side))
}

/// Moves a handle to the absolute point `(time, value)` for component `comp`.
///
/// A computed tangent mode switches to [`TangentMode::Aligned`] (the usual graph-editor
/// behaviour: grabbing an auto handle makes it manual but keeps the curve smooth). In aligned
/// mode the opposite handle keeps its length and follows the slope.
pub fn set_handle_point(
    channel: &mut Channel,
    key: usize,
    comp: usize,
    side: HandleSide,
    time: f32,
    value: f32,
) {
    fn h<T: ScalarView>(
        c: &mut Curve<T>,
        key: usize,
        comp: usize,
        side: HandleSide,
        time: f32,
        value: f32,
    ) {
        if !T::HANDLES {
            return;
        }
        let Some(k) = c.key_mut(key) else { return };
        if k.tangent_mode.is_computed() {
            k.tangent_mode = TangentMode::Aligned;
        }
        let v = T::get(&k.value, comp);
        let (dt, dv) = match side {
            HandleSide::In => ((k.time - time).max(0.0), v - value),
            HandleSide::Out => ((time - k.time).max(0.0), value - v),
        };
        let slope = if dt > 1e-6 { dv / dt } else { 0.0 };
        let set = |t: &mut Tangent<T>, dt: f32, dv: f32| {
            t.time = dt;
            T::set_delta(&mut t.value, comp, dv);
        };
        match side {
            HandleSide::In => set(&mut k.in_tangent, dt, dv),
            HandleSide::Out => set(&mut k.out_tangent, dt, dv),
        }
        if k.tangent_mode == TangentMode::Aligned {
            match side {
                HandleSide::In => {
                    let len = k.out_tangent.time;
                    set(&mut k.out_tangent, len, slope * len);
                }
                HandleSide::Out => {
                    let len = k.in_tangent.time;
                    set(&mut k.in_tangent, len, slope * len);
                }
            }
        }
        c.recompute_tangents();
    }
    with_curve!(channel, c => h(c, key, comp, side, time, value));
}
