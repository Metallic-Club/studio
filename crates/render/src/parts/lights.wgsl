// Local lights (lights/mod.rs): point and spot lights, clustered, with shadows from the atlas. Appended to
// parts.wgsl; `shade` there adds `local_lighting`. Everything here is in uniforms and textures, never in storage
// buffers, so it runs the same on GLES-class devices whose fragment shaders cannot read storage buffers.

// The cluster grid (CLUSTERS_* in lights/cluster.rs) and the cluster texture's width.
const CLUSTERS_X: u32 = 16u;
const CLUSTERS_Y: u32 = 9u;
const CLUSTERS_Z: u32 = 24u;
const CLUSTER_COUNT: u32 = 3456u;
const CLUSTER_TEXTURE_WIDTH: u32 = 1024u;
// MIN_DISTANCE² in lights/mod.rs.
const MIN_DISTANCE_SQUARED: f32 = 0.01;
// MAX_RECEIVER_OFFSET in shadow/mod.rs.
const MAX_RECEIVER_OFFSET: f32 = 0.05;

// A constant per pipeline (parts/mod.rs makes the variants and picks one per frame): whether it shades the local
// lights' shadows. Code that is there costs every pixel registers, and so occupancy, even in frames that never run
// it: measured on `many_lights`, the shadow lookup costs a quarter of the light loop's time with no shadowed light
// drawn. (Frames without local lights use lights_off.wgsl instead of this file: the loop alone costs an unlit
// frame's opaque pass 0.08 to 0.16 ms.)
override LOCAL_SHADOWS: bool = true;

// `GpuLight` in lights/gpu.rs.
struct LocalLight {
    // xyz: camera-relative position; w: range.
    position: vec4<f32>,
    // rgb: intensity (colour × Brightness × LIGHT_INTENSITY × fade).
    color: vec4<f32>,
    // xyz: where a spot points; w: one minus the cosine of its half-angle, −2 for a point light.
    direction: vec4<f32>,
    // x: the cone's falloff scale; y: 1 when the shadow is a cube of six tiles (a point light, or a spot wider than one
    // tile covers); z: the first shadow tile (−1: none); w: how much shadow.
    params: vec4<f32>,
}

// `GpuTile` in lights/atlas.rs.
struct ShadowTile {
    // Camera-relative positions to the tile's clip space.
    clip: mat4x4<f32>,
    // xy: the tile's corner in the atlas; z: its size.
    rect: vec4<f32>,
    // x: near × far / (far − near); y: a texel's width per metre of depth.
    params: vec4<f32>,
}

@group(0) @binding(4) var<uniform> local_lights: array<LocalLight, 256>;
@group(0) @binding(5) var light_clusters: texture_2d<u32>;
@group(0) @binding(6) var<uniform> shadow_tiles: array<ShadowTile, 64>;
@group(0) @binding(7) var shadow_atlas: texture_depth_2d;

fn cluster_texel(i: u32) -> u32 {
    return textureLoad(light_clusters, vec2<i32>(i32(i % CLUSTER_TEXTURE_WIDTH), i32(i / CLUSTER_TEXTURE_WIDTH)), 0).r;
}

// The cluster of a fragment at `frag` (pixels from the top left) and view depth `depth`.
fn cluster_index(frag: vec2<f32>, depth: f32) -> u32 {
    let x = min(u32(max(frag.x * frame.lights_clusters.x, 0.0)), CLUSTERS_X - 1u);
    let y = min(u32(max(frag.y * frame.lights_clusters.y, 0.0)), CLUSTERS_Y - 1u);
    var z = 0u;
    if depth >= frame.lights_local.y {
        z = min(u32(max(log2(depth) * frame.lights_clusters.z + frame.lights_clusters.w, 0.0)) + 1u, CLUSTERS_Z - 1u);
    }
    return (z * CLUSTERS_Y + y) * CLUSTERS_X + x;
}

// How much of a shadowed light reaches `p` (camera-relative, unit normal `n` at `n_dot_l` to the light),
// `from_light` from the light at distance `d`: 1 lit, 0 in shadow. A point light's tile is the cube face of
// `from_light`'s major axis, in the order +X, −X, +Y, −Y, +Z, −Z (`FACES` in lights/atlas.rs).
fn local_visibility(light: LocalLight, p: vec3<f32>, n: vec3<f32>, n_dot_l: f32, from_light: vec3<f32>, d: f32) -> f32 {
    var index = u32(light.params.z);
    if light.params.y > 0.5 {
        let a = abs(from_light);
        if a.x >= a.y && a.x >= a.z {
            index += select(1u, 0u, from_light.x > 0.0);
        } else if a.y >= a.z {
            index += select(3u, 2u, from_light.y > 0.0);
        } else {
            index += select(5u, 4u, from_light.z > 0.0);
        }
    }
    let tile = shadow_tiles[min(index, 63u)];
    // The offsets are in texels of the tile at this distance, at most MAX_RECEIVER_OFFSET metres however coarse the
    // tile; the normal offset only for receivers at a grazing angle to the light (LightSettings in lights/mod.rs).
    let texel_size = tile.params.y * d;
    let offset = min(frame.lights_local.z * texel_size * (1.0 - n_dot_l), MAX_RECEIVER_OFFSET);
    let clip = tile.clip * vec4<f32>(p + n * offset, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    // The depth bias in metres at this depth, in depth units.
    let bias = min(frame.lights_local.w * texel_size, MAX_RECEIVER_OFFSET);
    let reference = ndc.z - bias * tile.params.x / (clip.w * clip.w);
    let texel = frame.lights_atlas.x;
    let low = tile.rect.xy + vec2<f32>(texel);
    let high = tile.rect.xy + vec2<f32>(tile.rect.z - texel);
    let uv = tile.rect.xy + vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * tile.rect.z;
    // Four bilinear comparison taps half a texel apart (a tent over 3 × 3 texels), or one with the filter off. Fixed
    // taps rather than a loop over a reach read from the frame: a little cheaper inside the light loop.
    let spread = frame.lights_atlas.y * 0.5 * texel;
    var lit = textureSampleCompareLevel(shadow_atlas, shadow_sampler, clamp(uv + vec2<f32>(-spread, -spread), low, high), reference);
    lit += textureSampleCompareLevel(shadow_atlas, shadow_sampler, clamp(uv + vec2<f32>(spread, -spread), low, high), reference);
    lit += textureSampleCompareLevel(shadow_atlas, shadow_sampler, clamp(uv + vec2<f32>(-spread, spread), low, high), reference);
    lit += textureSampleCompareLevel(shadow_atlas, shadow_sampler, clamp(uv + vec2<f32>(spread, spread), low, high), reference);
    return lit * 0.25;
}

// The light the local lights reflect at a point: camera-relative `p`, unit normal `n`, towards the camera `v`, seen
// at fragment `frag`. `diffuse`, `f0` and `a2` (roughness⁴) are the surface's, as in `shade`. Each light adds
// Lambert diffuse and GGX specular of its radiance, inverse-square windowed to reach 0 at its range, through its cone
// for a spot, and through its shadow when it has one.
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
    if frame.lights_local.x < 0.5 {
        return out;
    }
    let header = cluster_texel(cluster_index(frag, dot(p, frame.view_forward.xyz)));
    let first = header >> 8u;
    let count = header & 255u;
    for (var k = 0u; k < count; k += 1u) {
        let at = first + k;
        let light = local_lights[(cluster_texel(CLUSTER_COUNT + at / 4u) >> ((at % 4u) * 8u)) & 255u];
        let to_light = light.position.xyz - p;
        let d2 = dot(to_light, to_light);
        let range2 = light.position.w * light.position.w;
        if d2 >= range2 {
            continue;
        }
        let d = sqrt(d2);
        let l = to_light / max(d, 1e-4);
        let n_dot_l = dot(n, l);
        if n_dot_l <= 0.0 {
            continue;
        }
        let ratio = d2 / range2;
        let window = clamp(1.0 - ratio * ratio, 0.0, 1.0);
        var attenuation = window * window / max(d2, MIN_DISTANCE_SQUARED);
        if light.direction.w > -1.5 {
            // 1 − cos of the angle from the cone's axis, from the distance between the two unit vectors: precise
            // however narrow the cone (#404), where comparing cosines a hair below 1 flickered.
            let apart = -l - light.direction.xyz;
            let off_axis = 0.5 * dot(apart, apart);
            let cone = clamp((light.direction.w - off_axis) * light.params.x, 0.0, 1.0);
            attenuation *= cone * cone;
        }
        if attenuation <= 0.0 {
            continue;
        }
        if LOCAL_SHADOWS && light.params.w > 0.0 {
            attenuation *= mix(1.0, local_visibility(light, p, n, n_dot_l, -to_light, d), light.params.w);
        }
        let radiance = light.color.rgb * (attenuation * n_dot_l);
        // As in `shade`: l + v is 0 when the light is straight behind the point on the view ray (n·l > 0 with an
        // interpolated normal turned from the camera), and the cosines stay within 0..1 for pow.
        let h = unit_or_zero(l + v);
        let n_dot_h = clamp(dot(n, h), 0.0, 1.0);
        let v_dot_h = clamp(dot(v, h), 0.0, 1.0);
        let d_denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
        let distribution = a2 / (PI * d_denominator * d_denominator);
        let visibility = 0.5 / max(
            n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2) + n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2),
            1e-5,
        );
        let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
        out.body += diffuse / PI * radiance;
        out.specular += distribution * visibility * fresnel * radiance;
    }
    return out;
}
