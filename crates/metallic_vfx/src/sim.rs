//! The deterministic CPU reference simulator.
//!
//! [`Simulation`] runs one [`Effect`]. Given the same effect, seed and sequence of `step(dt)` calls it
//! produces bit-identical particles on every machine: all randomness is counter-based ([`crate::rng`]),
//! no clocks or hash maps are involved and particles are processed in a fixed order.
//!
//! Particles live in a structure-of-arrays [`ParticlePool`] whose streams are `vec4<f32>` each, the
//! layout a compute shader port reads directly (see [`crate::gpu`]).
//!
//! Order inside one emitter step:
//! 1. age particles and kill the expired ones;
//! 2. apply forces (update modules, top to bottom), integrate, collide;
//! 3. spawn (rate, bursts, distance), each new particle placed at its exact sub-step birth time along
//!    the effect's motion so fast-moving emitters leave a continuous stream;
//! 4. evaluate over-life modules into the derived `visual` and `tint` streams;
//! 5. compact dead particles away (swap-remove) and update bounds.

use crate::VfxError;
use crate::color::Rgb;
use crate::curve::{Curve, Gradient, Range};
use crate::effect::{
    ArtStyle, Direction, Effect, Emitter, HitAction, InOut, InitModule, MAX_LIFETIME, Shape,
    SimSpace, SpawnModule, Trail, UpdateModule,
};
use crate::math::{Transform, Vec3};
use crate::noise::curl3;
use crate::rng::Rng;
use std::collections::VecDeque;

/// Longest step accepted; larger `dt` values are clamped so a hitch cannot explode the simulation.
pub const MAX_STEP: f32 = 0.25;

/// Particles of one emitter, structure-of-arrays. Every stream has one `[f32; 4]` per particle, so a
/// GPU port binds each as `array<vec4<f32>>` with no padding rules to worry about.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParticlePool {
    /// `xyz` position (world, or local for local-space emitters), `w` age in seconds.
    pub position_age: Vec<[f32; 4]>,
    /// `xyz` velocity in studs/s, `w` lifetime in seconds.
    pub velocity_life: Vec<[f32; 4]>,
    /// `rgb` starting colour (sRGB), `w` starting size in studs.
    pub base: Vec<[f32; 4]>,
    /// `x` rotation in radians, `y` spin in rad/s, `z` envelope noise in `-1..=1`, `w` random `0..1`.
    pub motion: Vec<[f32; 4]>,
    /// Derived each step: `x` width, `y` height, `z` erosion `0..=1`, `w` reserved (0).
    pub visual: Vec<[f32; 4]>,
    /// Derived each step: `rgb` colour (sRGB), `a` opacity.
    pub tint: Vec<[f32; 4]>,
    /// Spawn order id, unique per emitter run; ribbons connect particles by it.
    pub ids: Vec<u32>,
}

impl ParticlePool {
    /// Live particle count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when no particle is alive.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn reserve(&mut self, n: usize) {
        self.position_age.reserve(n);
        self.velocity_life.reserve(n);
        self.base.reserve(n);
        self.motion.reserve(n);
        self.visual.reserve(n);
        self.tint.reserve(n);
        self.ids.reserve(n);
    }

    fn clear(&mut self) {
        self.position_age.clear();
        self.velocity_life.clear();
        self.base.clear();
        self.motion.clear();
        self.visual.clear();
        self.tint.clear();
        self.ids.clear();
    }

    fn swap_remove(&mut self, i: usize) {
        self.position_age.swap_remove(i);
        self.velocity_life.swap_remove(i);
        self.base.swap_remove(i);
        self.motion.swap_remove(i);
        self.visual.swap_remove(i);
        self.tint.swap_remove(i);
        self.ids.swap_remove(i);
    }

    fn truncate(&mut self, n: usize) {
        self.position_age.truncate(n);
        self.velocity_life.truncate(n);
        self.base.truncate(n);
        self.motion.truncate(n);
        self.visual.truncate(n);
        self.tint.truncate(n);
        self.ids.truncate(n);
    }

    /// Position of particle `i`.
    #[must_use]
    pub fn position(&self, i: usize) -> Vec3 {
        let p = self.position_age[i];
        Vec3::new(p[0], p[1], p[2])
    }

    /// Velocity of particle `i`.
    #[must_use]
    pub fn velocity(&self, i: usize) -> Vec3 {
        let v = self.velocity_life[i];
        Vec3::new(v[0], v[1], v[2])
    }
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

impl Aabb {
    /// Grows the box to hold a sphere.
    #[must_use]
    pub fn union_sphere(self, c: Vec3, r: f32) -> Self {
        Self {
            min: self.min.min(c - Vec3::splat(r)),
            max: self.max.max(c + Vec3::splat(r)),
        }
    }

    /// Box around a sphere.
    #[must_use]
    pub fn from_sphere(c: Vec3, r: f32) -> Self {
        Self {
            min: c - Vec3::splat(r),
            max: c + Vec3::splat(r),
        }
    }

    /// Smallest box holding both.
    #[must_use]
    pub fn union(self, o: Self) -> Self {
        Self {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    /// True when `p` is inside (inclusive).
    #[must_use]
    pub fn contains(&self, p: Vec3) -> bool {
        p.x >= self.min.x
            && p.y >= self.min.y
            && p.z >= self.min.z
            && p.x <= self.max.x
            && p.y <= self.max.y
            && p.z <= self.max.z
    }
}

/// Runtime state of one emitter.
#[derive(Debug, Clone, Default)]
pub struct EmitterState {
    /// The particles.
    pub pool: ParticlePool,
    /// Emitter clock in seconds (scaled by `time_scale`), including the delay.
    pub time: f64,
    /// Bounds of live particles including their size, or `None` when empty.
    pub bounds: Option<Aabb>,
    rate_acc: f64,
    dist_acc: f64,
    next_id: u32,
    fixed_acc: f32,
    /// Particles requested by [`Simulation::emit`], spawned on the next step.
    pending_emit: u32,
}

/// One point pair of a trail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    /// World position of attachment 0.
    pub a: Vec3,
    /// World position of attachment 1.
    pub b: Vec3,
    /// Seconds since the point was laid down.
    pub age: f32,
}

/// Runtime state of one trail: newest point first.
#[derive(Debug, Clone, Default)]
pub struct TrailState {
    /// Points, head first. The head follows the attachments until the trail moves `min_length`.
    pub points: VecDeque<TrailPoint>,
}

/// Runs one effect on the CPU.
#[derive(Debug, Clone)]
pub struct Simulation {
    effect: Effect,
    seed: u32,
    time: f32,
    transform: Transform,
    prev_transform: Transform,
    velocity: Vec3,
    emitters: Vec<EmitterState>,
    trails: Vec<TrailState>,
}

/// Over-life modules resolved once per step.
struct OverLife<'a> {
    size: Option<&'a Curve>,
    color: Option<&'a Gradient>,
    transparency: Option<&'a Curve>,
    squash: Option<&'a Curve>,
    erosion: Option<&'a Curve>,
}

impl Simulation {
    /// Builds a simulation of `effect` with a random `seed`.
    ///
    /// # Errors
    /// [`VfxError::Invalid`] if the effect fails [`Effect::validate`].
    pub fn new(effect: Effect, seed: u32) -> Result<Self, VfxError> {
        effect.validate()?;
        let emitters = effect.emitters.iter().map(|_| EmitterState::default()).collect();
        let trails = effect.trails.iter().map(|_| TrailState::default()).collect();
        Ok(Self {
            effect,
            seed,
            time: 0.0,
            transform: Transform::IDENTITY,
            prev_transform: Transform::IDENTITY,
            velocity: Vec3::ZERO,
            emitters,
            trails,
        })
    }

    /// The effect being simulated.
    #[must_use]
    pub fn effect(&self) -> &Effect {
        &self.effect
    }

    /// Replaces the effect while keeping live particles (live editing). Emitters and trails are matched
    /// by index; added ones start empty, removed ones disappear.
    ///
    /// # Errors
    /// [`VfxError::Invalid`] if the new effect fails validation; the old one keeps running.
    pub fn set_effect(&mut self, effect: Effect) -> Result<(), VfxError> {
        effect.validate()?;
        self.emitters.resize_with(effect.emitters.len(), EmitterState::default);
        self.trails.resize_with(effect.trails.len(), TrailState::default);
        for (state, em) in self.emitters.iter_mut().zip(&effect.emitters) {
            if state.pool.len() > em.capacity as usize {
                state.pool.truncate(em.capacity as usize);
            }
        }
        self.effect = effect;
        Ok(())
    }

    /// The random seed.
    #[must_use]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Seconds simulated since the start.
    #[must_use]
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Current effect transform.
    #[must_use]
    pub fn transform(&self) -> Transform {
        self.transform
    }

    /// Effect velocity measured over the last step.
    #[must_use]
    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    /// Moves the effect; the next [`Simulation::step`] interpolates from the previous transform.
    pub fn set_transform(&mut self, t: Transform) {
        self.transform = t;
    }

    /// Moves the effect without interpolation (a teleport).
    pub fn teleport(&mut self, t: Transform) {
        self.transform = t;
        self.prev_transform = t;
    }

    /// Runtime state of every emitter.
    #[must_use]
    pub fn emitters(&self) -> &[EmitterState] {
        &self.emitters
    }

    /// Runtime state of every trail.
    #[must_use]
    pub fn trails(&self) -> &[TrailState] {
        &self.trails
    }

    /// Total live particles.
    #[must_use]
    pub fn particle_count(&self) -> usize {
        self.emitters.iter().map(|e| e.pool.len()).sum()
    }

    /// Bounds of every live particle and trail point, or `None` when nothing is alive.
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb> {
        let mut out: Option<Aabb> = None;
        for (state, em) in self.emitters.iter().zip(&self.effect.emitters) {
            if let Some(b) = state.bounds {
                let b = if em.space == SimSpace::Local {
                    local_bounds_to_world(b, &self.transform)
                } else {
                    b
                };
                out = Some(out.map_or(b, |o| o.union(b)));
            }
        }
        for trail in &self.trails {
            for p in &trail.points {
                let r = (p.a - p.b).length() * 0.5;
                let b = Aabb::from_sphere((p.a + p.b) * 0.5, r);
                out = Some(out.map_or(b, |o| o.union(b)));
            }
        }
        out
    }

    /// Restarts from time 0 with no particles (same seed, so the replay is identical).
    pub fn restart(&mut self) {
        for e in &mut self.emitters {
            *e = EmitterState::default();
        }
        for t in &mut self.trails {
            t.points.clear();
        }
        self.time = 0.0;
        self.prev_transform = self.transform;
        self.velocity = Vec3::ZERO;
    }

    /// Restarts with a new seed.
    pub fn reseed(&mut self, seed: u32) {
        self.seed = seed;
        self.restart();
    }

    /// Jumps to time `t` by restarting and stepping at `fixed_dt` (scrubbing).
    pub fn seek(&mut self, t: f32, fixed_dt: f32) {
        self.restart();
        let dt = if fixed_dt.is_finite() && fixed_dt > 1e-4 {
            fixed_dt
        } else {
            1.0 / 60.0
        };
        let target = if t.is_finite() { t.max(0.0) } else { 0.0 };
        let steps = (f64::from(target) / f64::from(dt)).round() as u64;
        for _ in 0..steps.min(1_000_000) {
            self.step(dt);
        }
    }

    /// Spawns `count` particles on emitter `index` at the next step (Roblox `ParticleEmitter:Emit`).
    ///
    /// # Errors
    /// [`VfxError::NoSuchEmitter`] when `index` is out of range.
    pub fn emit(&mut self, index: usize, count: u32) -> Result<(), VfxError> {
        let state = self
            .emitters
            .get_mut(index)
            .ok_or(VfxError::NoSuchEmitter(index))?;
        state.pending_emit = state.pending_emit.saturating_add(count);
        Ok(())
    }

    /// Removes every live particle and trail point (Roblox `Clear`); spawning continues.
    pub fn clear(&mut self) {
        for e in &mut self.emitters {
            e.pool.clear();
            e.bounds = None;
        }
        for t in &mut self.trails {
            t.points.clear();
        }
    }

    /// True for a one-shot effect that has finished spawning and has nothing left alive.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        if self.effect.looping {
            return false;
        }
        let spawning_done = self
            .effect
            .emitters
            .iter()
            .zip(&self.emitters)
            .all(|(em, st)| {
                !em.enabled || st.time - f64::from(em.delay) >= f64::from(self.effect.duration)
            });
        spawning_done
            && self.particle_count() == 0
            && self.trails.iter().all(|t| t.points.is_empty())
    }

    /// Advances the simulation by `dt` seconds. Non-finite or negative `dt` is treated as 0; values above
    /// [`MAX_STEP`] are clamped.
    pub fn step(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_STEP) } else { 0.0 };
        let from = self.prev_transform;
        let to = self.transform;
        if dt > 0.0 {
            self.velocity = (to.position - from.position) / dt;
        }
        let flat = self.effect.style == ArtStyle::Flat2D;
        for (index, (em, state)) in self
            .effect
            .emitters
            .iter()
            .zip(self.emitters.iter_mut())
            .enumerate()
        {
            let ctx = StepCtx {
                seed: self.seed,
                emitter_index: index as u32,
                duration: self.effect.duration,
                looping: self.effect.looping,
                velocity: self.velocity,
                flat,
                effect_time: self.time,
            };
            step_emitter(em, state, &ctx, dt, from, to);
        }
        for (trail, state) in self.effect.trails.iter().zip(self.trails.iter_mut()) {
            step_trail(trail, state, dt, &to);
        }
        self.time += dt;
        self.prev_transform = to;
    }
}

fn local_bounds_to_world(b: Aabb, t: &Transform) -> Aabb {
    let c = (b.min + b.max) * 0.5;
    let r = (b.max - b.min).length() * 0.5;
    Aabb::from_sphere(t.point(c), r)
}

struct StepCtx {
    seed: u32,
    emitter_index: u32,
    duration: f32,
    looping: bool,
    velocity: Vec3,
    flat: bool,
    effect_time: f32,
}

fn step_emitter(
    em: &Emitter,
    state: &mut EmitterState,
    ctx: &StepCtx,
    dt: f32,
    from: Transform,
    to: Transform,
) {
    let scaled = dt * em.time_scale.max(0.0);
    if em.fixed_fps > 0.0 {
        let h = 1.0 / em.fixed_fps;
        state.fixed_acc += scaled;
        let total = state.fixed_acc;
        let mut consumed = 0.0;
        let mut n = 0;
        while state.fixed_acc >= h && n < 64 {
            let f0 = if total > 0.0 { consumed / total } else { 1.0 };
            consumed += h;
            let f1 = if total > 0.0 { consumed / total } else { 1.0 };
            substep(em, state, ctx, h, from.lerp(&to, f0), from.lerp(&to, f1));
            state.fixed_acc -= h;
            n += 1;
        }
    } else {
        substep(em, state, ctx, scaled, from, to);
    }
}

fn substep(
    em: &Emitter,
    state: &mut EmitterState,
    ctx: &StepCtx,
    dt: f32,
    from: Transform,
    to: Transform,
) {
    let local = em.space == SimSpace::Local;
    update_particles(em, state, ctx, dt, &to, local);
    if em.enabled {
        spawn_particles(em, state, ctx, dt, from, to, local);
    } else {
        state.pending_emit = 0;
    }
    state.time += f64::from(dt);
    if ctx.flat {
        for (p, v) in state
            .pool
            .position_age
            .iter_mut()
            .zip(state.pool.velocity_life.iter_mut())
        {
            p[2] = 0.0;
            v[2] = 0.0;
        }
    }
    evaluate_visuals(em, state);
    compact_and_bound(state);
}

/// Ages, applies forces, integrates and collides existing particles.
fn update_particles(
    em: &Emitter,
    state: &mut EmitterState,
    ctx: &StepCtx,
    dt: f32,
    to: &Transform,
    local: bool,
) {
    let pool = &mut state.pool;
    if pool.is_empty() || dt <= 0.0 {
        return;
    }
    // Module points and axes are authored in local space; world-space emitters see them moved.
    let point = |p: Vec3| if local { p } else { to.point(p) };
    let dir = |d: Vec3| if local { d } else { to.rotate(d) };
    for (p, v) in pool.position_age.iter_mut().zip(&pool.velocity_life) {
        p[3] += dt;
        // Mark expired particles; they are removed after this step's pass.
        if p[3] >= v[3] {
            p[3] = f32::INFINITY;
        }
    }
    for m in &em.update {
        if m.is_disabled() {
            continue;
        }
        match m {
            UpdateModule::Acceleration { accel, .. } => {
                let a = *accel * dt;
                for v in &mut pool.velocity_life {
                    v[0] += a.x;
                    v[1] += a.y;
                    v[2] += a.z;
                }
            }
            UpdateModule::Drag { drag, .. } => {
                // Roblox semantics: speed halves every 1/drag seconds.
                let k = (-drag * dt * std::f32::consts::LN_2).exp();
                for v in &mut pool.velocity_life {
                    v[0] *= k;
                    v[1] *= k;
                    v[2] *= k;
                }
            }
            UpdateModule::CurlNoise {
                strength,
                scale,
                scroll,
                octaves,
                ..
            } => {
                let inv = 1.0 / scale.max(1e-3);
                let offset = *scroll * (ctx.effect_time * inv);
                let seed = ctx.seed ^ ctx.emitter_index.wrapping_mul(0x51ED_270B);
                for (p, v) in pool.position_age.iter().zip(pool.velocity_life.iter_mut()) {
                    let q = Vec3::new(p[0], p[1], p[2]) * inv - offset;
                    let c = curl3(q, seed, *octaves) * (strength * dt);
                    v[0] += c.x;
                    v[1] += c.y;
                    v[2] += c.z;
                }
            }
            UpdateModule::Vortex {
                axis,
                center,
                swirl,
                pull,
                ..
            } => {
                let a = dir(*axis).normalize_or(Vec3::Y);
                let c = point(*center);
                for (p, v) in pool.position_age.iter().zip(pool.velocity_life.iter_mut()) {
                    let r = Vec3::new(p[0], p[1], p[2]) - c;
                    let radial = r - a * r.dot(a);
                    let rn = radial.normalize_or(Vec3::ZERO);
                    let tangent = a.cross(rn);
                    let acc = (tangent * *swirl - rn * *pull) * dt;
                    v[0] += acc.x;
                    v[1] += acc.y;
                    v[2] += acc.z;
                }
            }
            UpdateModule::Attractor {
                position,
                strength,
                radius,
                kill_radius,
                ..
            } => {
                let target = point(*position);
                let r2 = radius * radius;
                let k2 = kill_radius * kill_radius;
                for (p, v) in pool
                    .position_age
                    .iter_mut()
                    .zip(pool.velocity_life.iter_mut())
                {
                    let d = target - Vec3::new(p[0], p[1], p[2]);
                    let d2 = d.length_squared();
                    if d2 < k2 {
                        p[3] = f32::INFINITY;
                        continue;
                    }
                    let falloff = 1.0 / (1.0 + d2 / r2);
                    let acc = d.normalize_or(Vec3::ZERO) * (strength * falloff * dt);
                    v[0] += acc.x;
                    v[1] += acc.y;
                    v[2] += acc.z;
                }
            }
            UpdateModule::SpeedLimit { max, .. } => {
                for v in &mut pool.velocity_life {
                    let s2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                    if s2 > max * max {
                        let k = max / s2.sqrt();
                        v[0] *= k;
                        v[1] *= k;
                        v[2] *= k;
                    }
                }
            }
            _ => {}
        }
    }
    for (p, v) in pool.position_age.iter_mut().zip(&pool.velocity_life) {
        p[0] += v[0] * dt;
        p[1] += v[1] * dt;
        p[2] += v[2] * dt;
    }
    for m in &em.update {
        if let UpdateModule::CollisionPlane {
            point: pp,
            normal,
            bounce,
            friction,
            on_hit,
            disabled: false,
        } = m
        {
            let n = dir(*normal).normalize_or(Vec3::Y);
            let o = point(*pp);
            for (p, v) in pool
                .position_age
                .iter_mut()
                .zip(pool.velocity_life.iter_mut())
            {
                let pos = Vec3::new(p[0], p[1], p[2]);
                let depth = (pos - o).dot(n);
                if depth >= 0.0 {
                    continue;
                }
                let vel = Vec3::new(v[0], v[1], v[2]);
                let fixed = pos - n * depth;
                p[0] = fixed.x;
                p[1] = fixed.y;
                p[2] = fixed.z;
                let vn = n * vel.dot(n);
                let vt = vel - vn;
                let out = match on_hit {
                    HitAction::Bounce => {
                        vt * (1.0 - friction.clamp(0.0, 1.0)) - vn * bounce.max(0.0)
                    }
                    HitAction::Stick => Vec3::ZERO,
                    HitAction::Kill => {
                        p[3] = f32::INFINITY;
                        vel
                    }
                };
                v[0] = out.x;
                v[1] = out.y;
                v[2] = out.z;
            }
        }
    }
    for m in &mut pool.motion {
        m[0] += m[1] * dt;
    }
}

/// Births new particles from rate, bursts, distance and `emit` requests.
fn spawn_particles(
    em: &Emitter,
    state: &mut EmitterState,
    ctx: &StepCtx,
    dt: f32,
    from: Transform,
    to: Transform,
    local: bool,
) {
    // (time fraction inside this step at which each particle is born)
    let mut births: Vec<f32> = Vec::new();
    for _ in 0..state.pending_emit {
        births.push(1.0);
    }
    state.pending_emit = 0;

    let t0 = state.time - f64::from(em.delay);
    let t1 = t0 + f64::from(dt);
    let duration = f64::from(ctx.duration);
    let limited = !ctx.looping && duration > 0.0;
    let start = t0.max(0.0);
    let end = if limited { t1.min(duration) } else { t1 };
    if end > start && dt > 0.0 {
        let span = end - start;
        let frac_of = |t: f64| ((t - t0) / f64::from(dt)).clamp(0.0, 1.0) as f32;
        for m in &em.spawn {
            if m.is_disabled() {
                continue;
            }
            match m {
                SpawnModule::Rate { per_second, .. } => {
                    let rate = f64::from(*per_second);
                    let before = state.rate_acc;
                    state.rate_acc += rate * span;
                    let n = (state.rate_acc + 1e-6).floor().max(0.0);
                    state.rate_acc -= n;
                    for k in 0..n as u64 {
                        // Exact crossing time of the (k+1)-th integer of the accumulator.
                        let need = (k as f64 + 1.0 - before).max(0.0);
                        let t = if rate > 0.0 { start + need / rate } else { end };
                        births.push(frac_of(t.min(end)));
                    }
                }
                SpawnModule::Burst {
                    time,
                    count,
                    cycles,
                    interval,
                    ..
                } => {
                    for c in 0..(*cycles).max(1) {
                        let fire = f64::from(*time) + f64::from(c) * f64::from(*interval);
                        if fires_in(fire, start, end, ctx.looping, duration) {
                            for _ in 0..*count {
                                births.push(frac_of(fire_time_in(fire, start, duration, ctx)));
                            }
                        }
                    }
                }
                SpawnModule::Distance { per_stud, .. } => {
                    let moved = f64::from((to.position - from.position).length());
                    let before = state.dist_acc;
                    state.dist_acc += moved * f64::from(*per_stud);
                    let n = (state.dist_acc + 1e-6).floor().max(0.0);
                    state.dist_acc -= n;
                    let gained = state.dist_acc + n - before;
                    for k in 0..n as u64 {
                        let need = (k as f64 + 1.0 - before).max(0.0);
                        let f = if gained > 0.0 { need / gained } else { 1.0 };
                        births.push(f.clamp(0.0, 1.0) as f32);
                    }
                }
            }
        }
    }
    if births.is_empty() {
        return;
    }
    let free = (em.capacity as usize).saturating_sub(state.pool.len());
    if births.len() > free {
        births.truncate(free);
    }
    state.pool.reserve(births.len());
    for f in births {
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let at = from.lerp(&to, f);
        let age = (1.0 - f) * dt;
        init_particle(em, state, ctx, id, &at, age, local);
    }
}

/// True if a burst at cycle time `fire` falls in `[start, end)` of emitter time.
fn fires_in(fire: f64, start: f64, end: f64, looping: bool, duration: f64) -> bool {
    if looping && duration > 0.0 {
        if fire >= duration {
            return false;
        }
        // First cycle index whose fire time is >= start.
        let k = ((start - fire) / duration).ceil().max(0.0);
        let t = fire + k * duration;
        t >= start && t < end
    } else {
        fire >= start && fire < end
    }
}

/// Emitter time at which the burst fires inside `[start, end)`.
fn fire_time_in(fire: f64, start: f64, duration: f64, ctx: &StepCtx) -> f64 {
    if ctx.looping && duration > 0.0 {
        let k = ((start - fire) / duration).ceil().max(0.0);
        fire + k * duration
    } else {
        fire
    }
}

/// Rotates by Euler angles in degrees: X, then Y, then Z.
fn euler(v: Vec3, deg: Vec3) -> Vec3 {
    if deg == Vec3::ZERO {
        return v;
    }
    let v = rotate_about(v, Vec3::X, deg.x.to_radians());
    let v = rotate_about(v, Vec3::Y, deg.y.to_radians());
    rotate_about(v, Vec3::Z, deg.z.to_radians())
}

fn rotate_about(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// Samples a shape: local position and outward normal.
fn sample_shape(shape: &Shape, surface: bool, rng: &mut Rng) -> (Vec3, Vec3) {
    match shape {
        Shape::Point => (Vec3::ZERO, rng.unit_vector()),
        Shape::Sphere { radius, partial } => {
            let p = partial.clamp(0.0, 1.0);
            let y = 1.0 - 2.0 * p * rng.next_f32();
            let phi = rng.next_f32() * std::f32::consts::TAU;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let n = Vec3::new(r * phi.cos(), y, r * phi.sin());
            let d = if surface {
                *radius
            } else {
                radius * rng.next_f32().cbrt()
            };
            (n * d, n)
        }
        Shape::Box { size } => {
            let h = *size * 0.5;
            if surface {
                let ax = (size.y * size.z).abs();
                let ay = (size.x * size.z).abs();
                let az = (size.x * size.y).abs();
                let total = (ax + ay + az).max(1e-6);
                let pick = rng.next_f32() * total;
                let sign = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
                let (u, v) = (rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                if pick < ax {
                    (
                        Vec3::new(sign * h.x, u * h.y, v * h.z),
                        Vec3::new(sign, 0.0, 0.0),
                    )
                } else if pick < ax + ay {
                    (
                        Vec3::new(u * h.x, sign * h.y, v * h.z),
                        Vec3::new(0.0, sign, 0.0),
                    )
                } else {
                    (
                        Vec3::new(u * h.x, v * h.y, sign * h.z),
                        Vec3::new(0.0, 0.0, sign),
                    )
                }
            } else {
                let p = Vec3::new(
                    rng.range(-h.x, h.x),
                    rng.range(-h.y, h.y),
                    rng.range(-h.z, h.z),
                );
                (p, p.normalize_or(Vec3::Y))
            }
        }
        Shape::Disc { radius, inner } => {
            let i = inner.clamp(0.0, 1.0);
            let u = if surface {
                1.0
            } else {
                (i * i + (1.0 - i * i) * rng.next_f32()).sqrt()
            };
            let phi = rng.next_f32() * std::f32::consts::TAU;
            let radial = Vec3::new(phi.cos(), 0.0, phi.sin());
            (radial * (radius * u), Vec3::Y)
        }
        Shape::Cylinder {
            radius,
            height,
            top,
        } => {
            let hy = rng.next_f32();
            let r_here = radius * (1.0 + (top - 1.0) * hy);
            let u = if surface {
                1.0
            } else {
                rng.next_f32().sqrt()
            };
            let phi = rng.next_f32() * std::f32::consts::TAU;
            let radial = Vec3::new(phi.cos(), 0.0, phi.sin());
            (
                radial * (r_here * u) + Vec3::Y * ((hy - 0.5) * height),
                radial,
            )
        }
        Shape::Cone { radius, angle } => {
            let u = rng.next_f32().sqrt();
            let phi = rng.next_f32() * std::f32::consts::TAU;
            let radial = Vec3::new(phi.cos(), 0.0, phi.sin());
            let frac = if surface { 1.0 } else { u };
            let tilt = angle.to_radians() * frac;
            let n = Vec3::Y * tilt.cos() + radial * tilt.sin();
            (radial * (radius * frac), n)
        }
        Shape::Edge { a, b } => {
            let t = rng.next_f32();
            let d = (*b - *a).normalize_or(Vec3::X);
            let n = (Vec3::Y - d * d.dot(Vec3::Y)).normalize_or(d.basis().0);
            (a.lerp(*b, t), n)
        }
        Shape::MeshSurface { .. } => {
            let n = rng.unit_vector();
            (n, n)
        }
    }
}

fn pick_color(choices: &[Rgb], mix: bool, rng: &mut Rng) -> Rgb {
    match choices.len() {
        0 => Rgb::WHITE,
        1 => choices[0],
        n => {
            let x = rng.next_f32() * if mix { (n - 1) as f32 } else { n as f32 };
            let i = (x.floor() as usize).min(n - 1);
            if mix && i + 1 < n {
                choices[i].lerp_oklab(choices[i + 1], x - i as f32)
            } else {
                choices[i]
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn init_particle(
    em: &Emitter,
    state: &mut EmitterState,
    ctx: &StepCtx,
    id: u32,
    at: &Transform,
    age: f32,
    local: bool,
) {
    let mut rng = Rng::for_particle(ctx.seed, ctx.emitter_index, id);
    let mut pos = Vec3::ZERO;
    let mut normal = Vec3::Y;
    let mut lifetime = Range::constant(2.0);
    let mut vel = Vec3::ZERO;
    let mut size = 1.0;
    let mut color = Rgb::WHITE;
    let mut rotation = 0.0;
    let mut spin = 0.0;
    for m in &em.init {
        if m.is_disabled() {
            continue;
        }
        match m {
            InitModule::Shape {
                shape,
                surface,
                rotation,
                offset,
                ..
            } => {
                let (p, n) = sample_shape(shape, *surface, &mut rng);
                let r = |v: Vec3| euler(v, *rotation);
                pos = r(p) + *offset;
                normal = r(n);
            }
            InitModule::Lifetime { seconds, .. } => lifetime = *seconds,
            InitModule::Velocity {
                speed,
                direction,
                spread,
                ..
            } => {
                let mut d = match direction {
                    Direction::Axis(a) => a.normalize_or(Vec3::Y),
                    Direction::FromShape(InOut::Outward) => normal,
                    Direction::FromShape(InOut::Inward) => -normal,
                    Direction::FromShape(InOut::Both) => {
                        if rng.next_f32() < 0.5 {
                            normal
                        } else {
                            -normal
                        }
                    }
                };
                let (t, b) = d.basis();
                let a1 = (rng.next_f32() - 0.5) * spread.0.to_radians();
                let a2 = (rng.next_f32() - 0.5) * spread.1.to_radians();
                d = rotate_about(d, t, a1);
                d = rotate_about(d, b, a2);
                vel += d * speed.at(rng.next_f32());
            }
            InitModule::Size { size: s, .. } => size = s.at(rng.next_f32()),
            InitModule::Color { choices, mix, .. } => color = pick_color(choices, *mix, &mut rng),
            InitModule::Rotation {
                angle, spin: sp, ..
            } => {
                rotation = angle.at(rng.next_f32()).to_radians();
                spin = sp.at(rng.next_f32()).to_radians();
            }
            InitModule::InheritVelocity { amount, .. } => {
                // `vel` is in the emitter's local frame here; it is rotated to world below for
                // world-space emitters, so the world velocity is first expressed in that frame.
                let w = ctx.velocity * *amount;
                vel += Vec3::new(w.dot(at.right), w.dot(at.up), w.dot(at.back));
            }
        }
    }
    let life = lifetime.at(rng.next_f32()).min(MAX_LIFETIME);
    if !(life > 0.0) {
        return;
    }
    let noise = rng.range(-1.0, 1.0);
    let random = rng.next_f32();
    let (p, v) = if local {
        (pos, vel)
    } else {
        (at.point(pos), at.rotate(vel))
    };
    let p = p + v * age;
    let pool = &mut state.pool;
    pool.position_age.push([p.x, p.y, p.z, age]);
    pool.velocity_life.push([v.x, v.y, v.z, life]);
    pool.base.push([color.r, color.g, color.b, size]);
    pool.motion.push([rotation + spin * age, spin, noise, random]);
    pool.visual.push([size, size, 0.0, 0.0]);
    pool.tint.push([color.r, color.g, color.b, 1.0]);
    pool.ids.push(id);
}

/// Computes the derived `visual` and `tint` streams from the over-life modules.
fn evaluate_visuals(em: &Emitter, state: &mut EmitterState) {
    let mut ol = OverLife {
        size: None,
        color: None,
        transparency: None,
        squash: None,
        erosion: None,
    };
    for m in &em.update {
        match m {
            UpdateModule::SizeOverLife {
                curve,
                disabled: false,
            } => ol.size = Some(curve),
            UpdateModule::ColorOverLife {
                gradient,
                disabled: false,
            } => ol.color = Some(gradient),
            UpdateModule::TransparencyOverLife {
                curve,
                disabled: false,
            } => ol.transparency = Some(curve),
            UpdateModule::SquashOverLife {
                curve,
                disabled: false,
            } => ol.squash = Some(curve),
            UpdateModule::ErosionOverLife {
                curve,
                disabled: false,
            } => ol.erosion = Some(curve),
            _ => {}
        }
    }
    let pool = &mut state.pool;
    for i in 0..pool.len() {
        let age = pool.position_age[i][3];
        let life = pool.velocity_life[i][3];
        let t = if life > 0.0 && age.is_finite() {
            (age / life).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let base = pool.base[i];
        let noise = pool.motion[i][2];
        let mut size = base[3];
        if let Some(c) = ol.size {
            size *= c.sample_with(t, noise).max(0.0);
        }
        let (mut w, mut h) = (size, size);
        if let Some(c) = ol.squash {
            let s = c.sample_with(t, noise).clamp(-3.0, 3.0);
            let k = s.exp2();
            w = size / k;
            h = size * k;
        }
        let erosion = ol
            .erosion
            .map_or(0.0, |c| c.sample_with(t, noise).clamp(0.0, 1.0));
        let mut rgb = Rgb::new(base[0], base[1], base[2]);
        if let Some(g) = ol.color {
            rgb = rgb.mul(g.sample(t));
        }
        let alpha = ol
            .transparency
            .map_or(1.0, |c| 1.0 - c.sample_with(t, noise).clamp(0.0, 1.0));
        pool.visual[i] = [w, h, erosion, 0.0];
        pool.tint[i] = [rgb.r, rgb.g, rgb.b, alpha];
    }
}

fn compact_and_bound(state: &mut EmitterState) {
    let pool = &mut state.pool;
    let mut i = 0;
    while i < pool.len() {
        let p = pool.position_age[i];
        let v = pool.velocity_life[i];
        let finite = p[0].is_finite() && p[1].is_finite() && p[2].is_finite() && v[0].is_finite();
        if !p[3].is_finite() || p[3] >= v[3] || !finite {
            pool.swap_remove(i);
        } else {
            i += 1;
        }
    }
    let mut bounds: Option<Aabb> = None;
    for (p, vis) in pool.position_age.iter().zip(&pool.visual) {
        let c = Vec3::new(p[0], p[1], p[2]);
        let r = vis[0].abs().max(vis[1].abs()) * std::f32::consts::FRAC_1_SQRT_2;
        bounds = Some(match bounds {
            Some(b) => b.union_sphere(c, r),
            None => Aabb::from_sphere(c, r),
        });
    }
    state.bounds = bounds;
}

fn step_trail(trail: &Trail, state: &mut TrailState, dt: f32, to: &Transform) {
    let life = trail.lifetime.clamp(0.01, MAX_LIFETIME);
    for p in &mut state.points {
        p.age += dt;
    }
    while state.points.back().is_some_and(|p| p.age >= life) {
        state.points.pop_back();
    }
    if trail.enabled {
        let a = to.point(trail.attachment0);
        let b = to.point(trail.attachment1);
        let head = TrailPoint { a, b, age: 0.0 };
        match state.points.len() {
            0 => {
                state.points.push_front(head);
                state.points.push_front(head);
            }
            1 => state.points.push_front(head),
            _ => {
                let anchor = state.points[1];
                let moved = ((a + b) * 0.5 - (anchor.a + anchor.b) * 0.5).length();
                if moved >= trail.min_length.max(1e-4) {
                    state.points.push_front(head);
                } else if let Some(front) = state.points.front_mut() {
                    *front = head;
                }
            }
        }
    }
    if trail.max_length > 0.0 {
        let mut total = 0.0;
        let mut keep = state.points.len();
        for i in 1..state.points.len() {
            let p = state.points[i - 1];
            let q = state.points[i];
            total += ((p.a + p.b) * 0.5 - (q.a + q.b) * 0.5).length();
            if total > trail.max_length {
                keep = i;
                break;
            }
        }
        state.points.truncate(keep);
    }
}
