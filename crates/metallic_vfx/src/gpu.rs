//! GPU-ready layout for a later compute port.
//!
//! The CPU simulator is the reference. A compute port binds the [`crate::ParticlePool`] streams as
//! `array<vec4<f32>>` (16-byte stride, so std430 adds no padding), uploads one [`GpuSimParams`] per
//! emitter per step, and bakes over-life curves and gradients into a small lookup table
//! ([`bake_over_life`]) so the shader never interprets curve keys. The matching WGSL is
//! [`WGSL_UPDATE`] (`shaders/particles_update.wgsl`), validated with naga in the tests.
//!
//! What stays on the CPU (assistant's call): spawn counting (rate accumulators, bursts, distance), the
//! effect transform and module flattening. That is a few bytes per emitter per frame and keeps the GPU side
//! a fixed kernel set, the Niagara "emitter on CPU, particles on GPU" split. Forces that depend on the
//! particle (curl noise, vortex, attractors, collision planes) become extra kernels or kernel branches;
//! `particles_sim.wgsl` in the render-styles research shows the dead-list and indirect-draw bookkeeping.

use crate::curve::Curve;
use crate::effect::{Emitter, UpdateModule};
use crate::math::Transform;

/// The reference update kernel and layout declarations in WGSL.
pub const WGSL_UPDATE: &str = include_str!("../shaders/particles_update.wgsl");

/// Entries per lookup table row in [`bake_over_life`].
pub const LUT_SIZE: usize = 64;

/// Per-emitter uniform block, byte-for-byte the WGSL `SimParams` struct (128 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuSimParams {
    /// Effect right vector, `w` 0.
    pub right: [f32; 4],
    /// Effect up vector, `w` 0.
    pub up: [f32; 4],
    /// Effect back vector, `w` 0.
    pub back: [f32; 4],
    /// Effect position, `w` 1.
    pub position: [f32; 4],
    /// `xyz` summed constant acceleration, `w` summed drag rate.
    pub accel_drag: [f32; 4],
    /// `x` dt, `y` emitter time, `z` LUT size, `w` speed limit (0 = none).
    pub step: [f32; 4],
    /// `x` live count, `y` spawn count, `z` first new id, `w` seed.
    pub counts: [u32; 4],
    /// Reserved.
    pub misc: [f32; 4],
}

impl GpuSimParams {
    /// Flattens the constant-force modules of an emitter into the uniform block.
    #[must_use]
    pub fn new(em: &Emitter, xf: &Transform, dt: f32, time: f32, live: u32, seed: u32) -> Self {
        let mut accel = [0.0f32; 3];
        let mut drag = 0.0;
        let mut limit = 0.0f32;
        for m in &em.update {
            if m.is_disabled() {
                continue;
            }
            match m {
                UpdateModule::Acceleration { accel: a, .. } => {
                    accel[0] += a.x;
                    accel[1] += a.y;
                    accel[2] += a.z;
                }
                UpdateModule::Drag { drag: d, .. } => drag += d,
                UpdateModule::SpeedLimit { max, .. } => {
                    limit = if limit > 0.0 { limit.min(*max) } else { *max };
                }
                _ => {}
            }
        }
        let v4 = |v: crate::math::Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(xf.right, 0.0),
            up: v4(xf.up, 0.0),
            back: v4(xf.back, 0.0),
            position: v4(xf.position, 1.0),
            accel_drag: [accel[0], accel[1], accel[2], drag],
            step: [dt, time, LUT_SIZE as f32, limit],
            counts: [live, 0, 0, seed],
            misc: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Bakes an emitter's over-life modules into `2 * LUT_SIZE` entries: first colour multiplier + opacity,
/// then `x` size multiplier, `y` squash, `z` erosion. Envelopes are not baked (the GPU port adds the
/// per-particle noise term separately).
#[must_use]
pub fn bake_over_life(em: &Emitter) -> Vec<[f32; 4]> {
    let mut size: Option<&Curve> = None;
    let mut squash: Option<&Curve> = None;
    let mut erosion: Option<&Curve> = None;
    let mut transparency: Option<&Curve> = None;
    let mut color = None;
    for m in &em.update {
        match m {
            UpdateModule::SizeOverLife {
                curve,
                disabled: false,
            } => size = Some(curve),
            UpdateModule::SquashOverLife {
                curve,
                disabled: false,
            } => squash = Some(curve),
            UpdateModule::ErosionOverLife {
                curve,
                disabled: false,
            } => erosion = Some(curve),
            UpdateModule::TransparencyOverLife {
                curve,
                disabled: false,
            } => transparency = Some(curve),
            UpdateModule::ColorOverLife {
                gradient,
                disabled: false,
            } => color = Some(gradient),
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(2 * LUT_SIZE);
    for i in 0..LUT_SIZE {
        let t = i as f32 / (LUT_SIZE - 1) as f32;
        let c = color.map_or(crate::color::Rgb::WHITE, |g| g.sample(t));
        let a = transparency.map_or(1.0, |c| 1.0 - c.sample(t).clamp(0.0, 1.0));
        out.push([c.r, c.g, c.b, a]);
    }
    for i in 0..LUT_SIZE {
        let t = i as f32 / (LUT_SIZE - 1) as f32;
        out.push([
            size.map_or(1.0, |c| c.sample(t).max(0.0)),
            squash.map_or(0.0, |c| c.sample(t)),
            erosion.map_or(0.0, |c| c.sample(t).clamp(0.0, 1.0)),
            0.0,
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_block_matches_wgsl_size() {
        assert_eq!(std::mem::size_of::<GpuSimParams>(), 128);
        assert_eq!(std::mem::align_of::<GpuSimParams>(), 4);
        assert_eq!(std::mem::size_of::<[f32; 4]>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let module = match naga::front::wgsl::parse_str(WGSL_UPDATE) {
            Ok(m) => m,
            Err(e) => panic!("{}", e.emit_to_string(WGSL_UPDATE)),
        };
        let mut v = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        if let Err(e) = v.validate(&module) {
            panic!("{e:?}");
        }
        // The uniform block in WGSL must be exactly as large as the Rust struct.
        let ty = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("SimParams"))
            .map(|(h, _)| h);
        let Some(ty) = ty else {
            panic!("SimParams missing");
        };
        let mut layouter = naga::proc::Layouter::default();
        if let Err(e) = layouter.update(module.to_ctx()) {
            panic!("{e:?}");
        }
        assert_eq!(layouter[ty].size as usize, std::mem::size_of::<GpuSimParams>());
    }

    #[test]
    fn baked_lut_matches_curves() {
        let em = crate::presets::by_name("Fire")
            .map(|e| e.emitters[0].clone())
            .unwrap_or_default();
        let lut = bake_over_life(&em);
        assert_eq!(lut.len(), 2 * LUT_SIZE);
        for entry in &lut {
            assert!(entry.iter().all(|v| v.is_finite()));
        }
    }
}
