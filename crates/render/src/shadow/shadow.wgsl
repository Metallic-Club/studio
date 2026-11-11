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

// See-through casters (#122, #491): the vertex entry points named `vs_see_through` hand the instance's Transparency to
// this fragment stage, which writes the share of light it lets through into the see-through light map
// (shadow/see_through.rs), blended by multiplying what is there: two see-through parts over one another let through
// what both do. Their depth goes to the see-through depth map with the solid casters' pipeline.
struct SeeThrough {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) transparency: f32,
}

@fragment
fn fs_see_through(in: SeeThrough) -> @location(0) vec4<f32> {
    return vec4<f32>(clamp(in.transparency, 0.0, 1.0));
}
