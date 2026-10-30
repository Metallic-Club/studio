// Full-screen vertical gradient sky with a soft-edged sun disc (no glow or bloom).
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
    var col = sky_gradient(dir);
    let disc = smoothstep(0.99975, 0.99991, dot(dir, g.sun_dir.xyz));
    col = mix(col, vec3<f32>(0.95, 0.92, 0.86), disc * 0.9);
    return vec4<f32>(finish(col, in.clip.xy), 1.0);
}
