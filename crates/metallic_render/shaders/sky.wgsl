// Full-screen sky: gradient, stars, moon, sun disc and fog (`sky_colour` in lighting.wgsl).
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> SkyOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    let p = uv * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_main(in: SkyOut) -> @location(0) vec4<f32> {
    let far_h = g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far_h.xyz / far_h.w - g.camera_pos.xyz);
    let col = sky_colour(dir);
    return vec4<f32>(col, 1.0);
}
