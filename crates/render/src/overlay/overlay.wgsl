// The debug overlay: screen-space quads, either solid or a 5x7 glyph whose pixels travel in the instance data.
// One instanced draw of six vertices per quad; no bind groups, no textures.

struct QuadIn {
    // Left, top, right, bottom in clip space.
    @location(0) rect: vec4<f32>,
    // sRGB-encoded colour with straight alpha.
    @location(1) color: vec4<f32>,
    // A glyph's 35 pixels (see font.rs), or unused for a solid quad.
    @location(2) bits: vec2<u32>,
    // 0: solid, 1: glyph.
    @location(3) kind: u32,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) bits: vec2<u32>,
    @location(3) @interpolate(flat) kind: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, quad: QuadIn) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    var out: VsOut;
    out.position = vec4<f32>(mix(quad.rect.xy, quad.rect.zw, corner), 0.0, 1.0);
    out.uv = corner;
    out.color = quad.color;
    out.bits = quad.bits;
    out.kind = quad.kind;
    return out;
}

fn coverage(in: VsOut) -> f32 {
    if in.kind == 0u {
        return 1.0;
    }
    let column = min(u32(in.uv.x * 5.0), 4u);
    let row = min(u32(in.uv.y * 7.0), 6u);
    let bit = row * 5u + column;
    var word = in.bits.x;
    var shift = bit;
    if bit >= 20u {
        word = in.bits.y;
        shift = bit - 20u;
    }
    return f32((word >> shift) & 1u);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// For targets that store what the shader writes (UNORM): the colour is already sRGB-encoded.
@fragment
fn fs_encoded(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, in.color.a * coverage(in));
}

// For sRGB targets, which encode on write: decode first.
@fragment
fn fs_linear(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a * coverage(in));
}
