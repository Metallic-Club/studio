// No local lights: appended to parts.wgsl in place of lights.wgsl for the pipelines of frames that draw none
// (parts/mod.rs). It declares none of the lights' bindings, so those pipelines bind only the sun's frame group: such
// a frame binds and draws exactly what it did before local lights existed.
fn local_lighting(
    frag: vec2<f32>,
    p: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    n_dot_v: f32,
    diffuse: vec3<f32>,
    f0: vec3<f32>,
    a2: f32,
) -> Shaded {
    var out: Shaded;
    out.body = vec3<f32>(0.0);
    out.specular = vec3<f32>(0.0);
    return out;
}
