// Depth-only pass from the sun's point of view, one cascade per draw.
@group(0) @binding(0) var<uniform> light_vp: mat4x4<f32>;

struct ShadowIn {
    @location(0) pos: vec3<f32>,
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
}

@vertex
fn vs_main(v: ShadowIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    return light_vp * (model * vec4<f32>(v.pos, 1.0));
}
