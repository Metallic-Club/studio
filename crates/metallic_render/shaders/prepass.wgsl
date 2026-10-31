// Geometry prepass for post edges (#65): single-sampled depth and one Rgba16Float target holding
// the world normal (rgb) and the linear view depth (a). Surfaces with their own inverted-hull
// outline (toon characters) store the depth negated, so the edge pass leaves them to the hull
// and draws no second line. Cleared to 0: background. Blended parts are not drawn.
struct PreOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) n: vec3<f32>,
}

struct PartIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
}

@vertex
fn vs_part(v: PartIn) -> PreOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let cof = mat3x3<f32>(cross(v.m1.xyz, v.m2.xyz), cross(v.m2.xyz, v.m0.xyz), cross(v.m0.xyz, v.m1.xyz));
    var out: PreOut;
    out.clip = g.view_proj * (model * vec4<f32>(v.pos, 1.0));
    out.n = cof * v.normal;
    return out;
}

@vertex
fn vs_skin(v: SkinIn) -> PreOut {
    let m = skin_matrix(v.joints, v.weights);
    var out: PreOut;
    out.clip = g.view_proj * (m * vec4<f32>(v.pos, 1.0));
    out.n = (m * vec4<f32>(v.normal, 0.0)).xyz;
    return out;
}

// Gets post edges.
@fragment
fn fs_edges(in: PreOut) -> @location(0) vec4<f32> {
    return vec4<f32>(normalize(in.n), 1.0 / in.clip.w);
}

// Outlined by its own hull: no post edges on it.
@fragment
fn fs_hull(in: PreOut) -> @location(0) vec4<f32> {
    return vec4<f32>(normalize(in.n), -1.0 / in.clip.w);
}
