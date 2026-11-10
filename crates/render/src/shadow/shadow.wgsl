// A shadow cascade's caster pass: depth only, seen along the light. Followed by shadow_storage.wgsl or
// shadow_vertex.wgsl, which read the instance as the parts' two paths do (scene/gpu.rs explains them).

// `CascadeUniform` in shadow/maps.rs.
struct Cascade {
    // Origin-relative positions to the map's clip space (depth 0 at the light's end).
    clip: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> cascade: Cascade;

// A mesh vertex placed with an instance's 3x4 affine (rows r0..r2), in the cascade's clip space.
fn shadow_place(position: vec3<f32>, r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>) -> vec4<f32> {
    let p = vec4<f32>(position, 1.0);
    return cascade.clip * vec4<f32>(dot(r0, p), dot(r1, p), dot(r2, p), 1.0);
}

// See-through casters (#122): the vertex entry points named `vs_see_through` hand the instance's Transparency and a
// seed to this fragment stage, which leaves out that share of the caster's texels. The comparison filter's taps then
// see the caster in about `1 − Transparency` of them, so a part that is a little see-through lets a little light
// through its shadow. Solid casters are drawn without a fragment stage (the pipeline without it keeps early depth).
struct SeeThrough {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) transparency: f32,
    @location(1) @interpolate(flat) seed: u32,
}

// A seed for an instance's texels from where it is (its rows' translation), the same on both instance paths.
fn see_through_seed(r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>) -> u32 {
    let h = bitcast<u32>(r0.w) ^ (bitcast<u32>(r1.w) * 0x9e3779b9u) ^ (bitcast<u32>(r2.w) * 0x85ebca6bu);
    return (h ^ (h >> 15u)) * 2654435761u;
}

// Interleaved gradient noise (Jimenez 2014): an even spread of thresholds over neighbouring texels, which the
// filter's 3 × 3 tent averages into a smooth share. Each instance shifts it, so two see-through parts over one
// another leave out different texels and let through what both do.
fn see_through_threshold(texel: vec2<f32>, seed: u32) -> f32 {
    let shift = vec2<f32>(f32(seed & 63u), f32((seed >> 6u) & 63u)) * 5.588238;
    let p = floor(texel) + shift;
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_see_through(in: SeeThrough) {
    if see_through_threshold(in.position.xy, in.seed) < in.transparency {
        discard;
    }
}
