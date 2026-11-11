// The vertex-buffer path of the caster pass (appended to shadow.wgsl), for GLES: the cascade's list holds copies of
// the instances, read as per-instance attributes (`GpuInstance::vertex_attributes(2)`; the rows, and for see-through
// casters the Transparency).

@vertex
fn vs_shadow(
    @location(0) position: vec3<f32>,
    @location(2) r0: vec4<f32>,
    @location(3) r1: vec4<f32>,
    @location(4) r2: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    return shadow_place(position, r0, r1, r2);
}

@vertex
fn vs_see_through(
    @location(0) position: vec3<f32>,
    @location(2) r0: vec4<f32>,
    @location(3) r1: vec4<f32>,
    @location(4) r2: vec4<f32>,
    @location(6) transparency: f32,
) -> SeeThrough {
    var out: SeeThrough;
    out.position = shadow_place(position, r0, r1, r2);
    out.transparency = transparency;
    return out;
}
