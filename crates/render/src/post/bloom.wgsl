// Bloom (`post/bloom.rs`): a chain of downsampling passes from the scene colour into ever smaller mips of the bloom
// texture, then upsampling passes back up, each adding its tent-filtered smaller mip onto the next larger one, so mip
// 0 ends up holding the sum of every level's blur. The finish pass mixes a share of it into the image.
//
// The first downsample reads the drawn corner of the scene colour and halves it (or quarters it, with four taps);
// with a threshold it keeps only what is brighter, with a soft knee. Later ones halve a mip with the dual filter's
// five taps or, at the higher effect qualities, the 13-tap filter of Jimenez's "Next generation post processing in
// Call of Duty" (2014), which flickers less on small bright things.
//
// Where the device has compute shaders the same filters run as compute dispatches instead (`bloom_compute.wgsl`,
// appended to this file), all of them in one compute pass (#145).

struct BloomPass {
    // xy: one source texel in texture coordinates; zw: the share of the source read (the drawn corner of the scene
    // colour; 1 for a mip).
    texel: vec4<f32>,
    // xy: the farthest source coordinate read, half a texel inside the region; z: the threshold (0: none); w: its
    // soft knee.
    limit: vec4<f32>,
}

@group(0) @binding(0) var<uniform> bloom: BloomPass;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // 0..1 over the destination, from the top left.
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_bloom(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// The source at `uv` (in the destination's 0..1) offset by `texels` source texels, kept inside the region read.
fn tap(uv: vec2<f32>, texels: vec2<f32>) -> vec3<f32> {
    let at = min(uv * bloom.texel.zw + texels * bloom.texel.xy, bloom.limit.xy);
    return finite(textureSampleLevel(source, linear_sampler, at, 0.0).rgb);
}

// `c` with every channel that is not a finite number made 0, the rest kept between 0 and the largest half float:
// one NaN or infinite pixel in the scene colour would otherwise spread through the blur to the whole image (the
// same as `finite` in finish.wgsl). Tested on the bits, which no compiler folds away.
fn finite(c: vec3<f32>) -> vec3<f32> {
    let bits = bitcast<vec3<u32>>(c);
    let ok = (bits & vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u);
    return select(vec3<f32>(0.0), clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0)), ok);
}

// What is brighter than the threshold, with a quadratic soft knee below it (none when the threshold is 0).
fn threshold(c: vec3<f32>) -> vec3<f32> {
    let t = bloom.limit.z;
    if t <= 0.0 {
        return c;
    }
    let knee = max(bloom.limit.w, 1e-4);
    let brightness = max(c.r, max(c.g, c.b));
    var soft = clamp(brightness - t + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    return c * (max(soft, brightness - t) / max(brightness, 1e-4));
}

// The dual filter's downsample: the centre (four texels, one bilinear tap) and the four diagonal texel pairs.
fn down_dual(uv: vec2<f32>) -> vec3<f32> {
    let centre = tap(uv, vec2<f32>(0.0));
    let corners = tap(uv, vec2<f32>(-1.0, -1.0)) + tap(uv, vec2<f32>(1.0, -1.0)) + tap(uv, vec2<f32>(-1.0, 1.0))
        + tap(uv, vec2<f32>(1.0, 1.0));
    return (centre * 4.0 + corners) * 0.125;
}

// The 13-tap downsample: five overlapping 2x2 boxes, the centre one weighted most.
fn down_wide(uv: vec2<f32>) -> vec3<f32> {
    let a = tap(uv, vec2<f32>(-2.0, -2.0));
    let b = tap(uv, vec2<f32>(0.0, -2.0));
    let c = tap(uv, vec2<f32>(2.0, -2.0));
    let d = tap(uv, vec2<f32>(-2.0, 0.0));
    let e = tap(uv, vec2<f32>(0.0, 0.0));
    let f = tap(uv, vec2<f32>(2.0, 0.0));
    let g = tap(uv, vec2<f32>(-2.0, 2.0));
    let h = tap(uv, vec2<f32>(0.0, 2.0));
    let i = tap(uv, vec2<f32>(2.0, 2.0));
    let j = tap(uv, vec2<f32>(-1.0, -1.0));
    let k = tap(uv, vec2<f32>(1.0, -1.0));
    let l = tap(uv, vec2<f32>(-1.0, 1.0));
    let m = tap(uv, vec2<f32>(1.0, 1.0));
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

// A quarter of the source in each direction: four bilinear taps, the 4x4 texels' mean.
fn down_quarter(uv: vec2<f32>) -> vec3<f32> {
    return (tap(uv, vec2<f32>(-1.0, -1.0)) + tap(uv, vec2<f32>(1.0, -1.0)) + tap(uv, vec2<f32>(-1.0, 1.0))
        + tap(uv, vec2<f32>(1.0, 1.0))) * 0.25;
}

@fragment
fn fs_first_dual(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(threshold(down_dual(in.uv)), 1.0);
}

@fragment
fn fs_first_wide(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(threshold(down_wide(in.uv)), 1.0);
}

@fragment
fn fs_first_quarter(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(threshold(down_quarter(in.uv)), 1.0);
}

@fragment
fn fs_down_dual(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(down_dual(in.uv), 1.0);
}

@fragment
fn fs_down_wide(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(down_wide(in.uv), 1.0);
}

// The upsample's 3x3 tent over the smaller mip.
fn up_tent(uv: vec2<f32>) -> vec3<f32> {
    let edges = tap(uv, vec2<f32>(0.0, -1.0)) + tap(uv, vec2<f32>(-1.0, 0.0)) + tap(uv, vec2<f32>(1.0, 0.0))
        + tap(uv, vec2<f32>(0.0, 1.0));
    let corners = tap(uv, vec2<f32>(-1.0, -1.0)) + tap(uv, vec2<f32>(1.0, -1.0)) + tap(uv, vec2<f32>(-1.0, 1.0))
        + tap(uv, vec2<f32>(1.0, 1.0));
    let sum = tap(uv, vec2<f32>(0.0)) * 4.0 + edges * 2.0 + corners;
    return sum * 0.0625;
}

// The upsample: the tent over the smaller mip, added onto the larger one by the pipeline's blend.
@fragment
fn fs_up(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(up_tent(in.uv), 1.0);
}
