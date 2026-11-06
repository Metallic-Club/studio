// The vertex-buffer path (appended to parts.wgsl), for GLES, whose vertex shaders cannot read storage buffers: the
// instance arrives as per-instance vertex attributes (`GpuInstance::vertex_attributes(2)`), read from the draw
// list, which on this path holds copies of the instances rather than slot entries.

// The draw's base-colour texture (parts/assets.rs): a mesh asset's, or white. Group 1 here: this path has no
// instance group.
@group(1) @binding(0) var base_texture: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) r0: vec4<f32>,
    @location(3) r1: vec4<f32>,
    @location(4) r2: vec4<f32>,
    @location(5) color: u32,
    @location(6) transparency: f32,
    @location(7) mesh_material: u32,
    @location(8) flags: u32,
    @location(9) uv: vec2<f32>,
    @builtin(vertex_index) vertex: u32,
) -> VertexOut {
    return place(pool_level(vertex), position, normal, uv, r0, r1, r2, color, transparency, mesh_material);
}
