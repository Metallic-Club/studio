// Metallic VFX: GPU particle layout and the reference update kernel.
//
// Mirrors `metallic_vfx::sim::ParticlePool` (one array<vec4<f32>> per stream, 16-byte stride, no padding
// rules) and `metallic_vfx::gpu::GpuSimParams` (64 + 64 bytes, std140/std430 compatible). The CPU
// simulator in `sim.rs` is the reference: a port must give the same results up to float rounding.
//
// Bookkeeping (dead list, ping-pong alive lists, counters, indirect args) follows
// docs/research/style-shaders/particles_sim.wgsl and is left to the renderer integration. This file
// shows the per-particle maths that has to match: ageing, constant forces, Roblox drag, integration and
// over-life lookups from a baked 64-entry LUT (`gpu::bake_over_life`).
// Validated by `cargo test -p metallic_vfx` (naga).

struct SimParams {
    // Effect transform: columns right, up, back, position (w = 1).
    right: vec4<f32>,
    up: vec4<f32>,
    back: vec4<f32>,
    position: vec4<f32>,
    // xyz summed acceleration (studs/s^2), w Roblox drag rate.
    accel_drag: vec4<f32>,
    // x dt, y emitter time, z lut size, w speed limit (0 = none).
    step: vec4<f32>,
    // x live count, y spawn count this step, z first new id, w seed.
    counts: vec4<u32>,
    // x size multiplier from the init stage, yzw reserved.
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: SimParams;
// Streams (ParticlePool): xyz position, w age.
@group(0) @binding(1) var<storage, read_write> position_age: array<vec4<f32>>;
// xyz velocity, w lifetime.
@group(0) @binding(2) var<storage, read_write> velocity_life: array<vec4<f32>>;
// rgb start colour, w start size.
@group(0) @binding(3) var<storage, read> base: array<vec4<f32>>;
// Derived: x width, y height, z erosion, w reserved; then rgb tint, a opacity (two vec4 per particle).
@group(0) @binding(4) var<storage, read_write> visual_tint: array<vec4<f32>>;
// Over-life LUT: entries [0, n) = rgb colour multiplier + opacity, [n, 2n) = x size, y squash, z erosion.
@group(0) @binding(5) var<storage, read> over_life: array<vec4<f32>>;

fn lut(offset: u32, t: f32) -> vec4<f32> {
    let n = u32(params.step.z);
    let x = clamp(t, 0.0, 1.0) * f32(n - 1u);
    let i = u32(floor(x));
    let j = min(i + 1u, n - 1u);
    return mix(over_life[offset + i], over_life[offset + j], fract(x));
}

@compute @workgroup_size(64)
fn cs_update(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.counts.x) {
        return;
    }
    let dt = params.step.x;
    var pa = position_age[i];
    var vl = velocity_life[i];
    pa.w = pa.w + dt;
    if (pa.w >= vl.w) {
        // Dead: the compaction pass returns the slot to the dead list.
        pa.w = 3.4e38;
        position_age[i] = pa;
        return;
    }
    var v = vl.xyz + params.accel_drag.xyz * dt;
    // Roblox drag: speed halves every 1/drag seconds.
    v = v * exp(-params.accel_drag.w * dt * 0.6931472);
    let limit = params.step.w;
    let speed = length(v);
    if (limit > 0.0 && speed > limit) {
        v = v * (limit / speed);
    }
    pa = vec4<f32>(pa.xyz + v * dt, pa.w);
    vl = vec4<f32>(v, vl.w);
    position_age[i] = pa;
    velocity_life[i] = vl;

    let t = pa.w / vl.w;
    let n = u32(params.step.z);
    let c = lut(0u, t);
    let s = lut(n, t);
    let size = base[i].w * s.x;
    let k = exp2(s.y);
    visual_tint[2u * i] = vec4<f32>(size / k, size * k, s.z, 0.0);
    visual_tint[2u * i + 1u] = vec4<f32>(base[i].rgb * c.rgb, c.a);
}
