// Shading of instanced parts with procedural material detail. One surface shader serves every
// shading model: each instance points at a row of the material table, and the row's model id
// picks Lit, Toon, Unlit or Flat (src/shading.rs), so mixing models needs no extra pipelines.

// One material table row (`MaterialRow` in src/shading.rs).
struct MaterialRow {
    surface: vec4<f32>, // x: model id, y: roughness (after the Look's floor), z: metalness, w: wrap
    toon: vec4<f32>,    // x: bands, y: softness, z: flatten, w: light step in EV (0 = off)
    tint: vec4<f32>,    // rgb: shade-side tint (linear), w: highlight size
    extra: vec4<f32>,   // x: highlight strength, y: rim width, z: rim colour 0xRRGGBB, w: roughness floor
}

// MAX_ROWS in src/shading.rs.
@group(0) @binding(3) var<uniform> materials: array<MaterialRow, 96>;

const MODEL_LIT: u32 = 0u;
const MODEL_TOON: u32 = 1u;
const MODEL_UNLIT: u32 = 2u;
const MODEL_FLAT: u32 = 3u;
// Row of the Character class in the place Look's rows (CHARACTER_ROW in src/shading.rs).
const CHARACTER_ROW: u32 = 10u;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
    @location(6) color: vec4<f32>,
    // x: material table row, y: flags, z: material id (procedural detail), w: reserved.
    @location(7) material: vec4<u32>,
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) local_p: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) color: vec4<f32>,
    // Same as VIn.material.
    @location(5) @interpolate(flat) material: vec4<u32>,
    // Roughness and metalness: the row's for parts, the slot's for skinned meshes.
    @location(6) @interpolate(flat) surf: vec2<f32>,
    // Unlit rows: the colour through `unfinish`, once per vertex (the inverse curve is a
    // bisection, too costly per pixel). Zero for other models.
    @location(7) @interpolate(flat) unlit: vec3<f32>,
    // Positive view depth, interpolated with perspective correction.
    @location(8) view_depth: f32,
}

// `VOut.unlit` for a colour shaded by `row`.
fn unlit_colour(row: u32, c: vec3<f32>) -> vec3<f32> {
    if (u32(material_row(row).surface.x + 0.5) == MODEL_UNLIT) {
        return unfinish(c);
    }
    return vec3<f32>(0.0);
}

fn material_row(i: u32) -> MaterialRow {
    return materials[min(i, 95u)];
}

@vertex
fn vs_main(v: VIn) -> VOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let wp = model * vec4<f32>(v.pos, 1.0);
    let c0 = v.m0.xyz;
    let c1 = v.m1.xyz;
    let c2 = v.m2.xyz;
    // The cofactor matrix is the inverse transpose up to a positive scale: correct under non-uniform scaling.
    let cof = mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1));
    var out: VOut;
    out.clip = g.view_proj * wp;
    out.view_depth = out.clip.w;
    out.world_pos = wp.xyz;
    out.world_n = normalize(cof * v.normal);
    out.local_p = v.pos * vec3<f32>(length(c0), length(c1), length(c2));
    out.local_n = v.normal;
    out.color = v.color;
    out.material = v.material;
    out.surf = material_row(v.material.x).surface.yz;
    out.unlit = unlit_colour(v.material.x, v.color.rgb);
    return out;
}

// ---- procedural detail -------------------------------------------------

fn hash13(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash13(i), hash13(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash13(i + vec3<f32>(0.0, 1.0, 0.0)), hash13(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash13(i + vec3<f32>(0.0, 0.0, 1.0)), hash13(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash13(i + vec3<f32>(0.0, 1.0, 1.0)), hash13(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    return 0.5 * vnoise(p) + 0.25 * vnoise(p * 2.03) + 0.125 * vnoise(p * 4.1) + 0.0625 * vnoise(p * 8.3);
}

// Noise value `v` of `freq` cycles per stud, faded to its mean 0.5 where one pixel covers `px`
// studs and the pattern would be finer than about two pixels. Unfiltered, such fine detail
// aliases into moire stripes at a distance (seen on Metal's brushed streaks).
fn fine(v: f32, freq: f32, px: f32) -> f32 {
    return mix(v, 0.5, smoothstep(0.25, 0.5, freq * px));
}

// Returns an albedo multiplier (rgb) and a roughness offset (w) for the material id. `px` is the
// size of one pixel on the surface, in studs.
fn detail(id: i32, p: vec3<f32>, n: vec3<f32>, px: f32) -> vec4<f32> {
    let an = abs(n);
    var q: vec2<f32>;
    if (an.x > an.y && an.x > an.z) {
        q = p.yz;
    } else if (an.y > an.z) {
        q = p.xz;
    } else {
        q = p.xy;
    }
    var a = vec3<f32>(1.0);
    var r = 0.0;
    switch id {
        case 0: { // Plastic: fine speckle
            let s = vnoise(p * 7.0);
            a = vec3<f32>(0.96 + 0.08 * s);
            r = 0.06 * (s - 0.5);
        }
        case 1: { // SmoothPlastic: nearly flat
            a = vec3<f32>(0.985 + 0.03 * vnoise(p * 3.0));
        }
        case 2: { // Metal: brushed streaks along local x
            let s = fine(vnoise(vec3<f32>(q.x * 0.5, q.y * 28.0, 0.5)), 28.0, px);
            let t = fine(vnoise(vec3<f32>(q.x * 2.0, q.y * 90.0, 1.5)), 90.0, px);
            a = vec3<f32>(0.86 + 0.2 * s + 0.06 * t);
            r = 0.14 * (s - 0.5);
        }
        case 3: { // Wood: growth rings with warp and fine pores
            let warp = fbm(vec3<f32>(q.x * 0.35, q.y * 1.1, 0.7)) * 3.0;
            let ring = fract(q.y * 1.7 + warp);
            let band = smoothstep(0.0, 0.55, ring) * (1.0 - smoothstep(0.55, 1.0, ring));
            let pores = fine(vnoise(vec3<f32>(q.x * 3.0, q.y * 40.0, 2.0)), 40.0, px);
            a = vec3<f32>(0.78 + 0.26 * band + 0.06 * pores);
            r = 0.1 * (pores - 0.5);
        }
        case 4: { // Concrete: mottling and pits
            let s = fbm(p * 1.6);
            let pits = smoothstep(0.78, 0.9, vnoise(p * 9.0));
            a = vec3<f32>(0.84 + 0.3 * s - 0.12 * pits);
            r = 0.06 * (s - 0.5);
        }
        case 5: { // Glass: clean
            a = vec3<f32>(1.0);
        }
        case 6: { // Grass: large patches plus fine blades
            let s = fbm(vec3<f32>(q.x, 0.0, q.y) * 0.7);
            let blades = fine(vnoise(vec3<f32>(q * 16.0, 3.0)), 16.0, px);
            let patch_tint = mix(vec3<f32>(0.8, 0.92, 0.74), vec3<f32>(1.12, 1.08, 0.88), s);
            a = patch_tint * (0.88 + 0.24 * blades);
        }
        case 7: { // Brick: running bond, 2 x 0.8 studs, with mortar
            let row = floor(q.y / 0.8);
            let off = 0.5 * (row - 2.0 * floor(row * 0.5));
            let bx = fract(q.x / 2.0 + off);
            let by = fract(q.y / 0.8);
            let d = min(min(bx, 1.0 - bx) * 2.0, min(by, 1.0 - by) * 0.8);
            let mortar = 1.0 - smoothstep(0.03, 0.08, d);
            let id2 = floor(q.x / 2.0 + off) + row * 17.0;
            let tint = 0.84 + 0.3 * hash13(vec3<f32>(id2, row, 5.0));
            let grain = 0.92 + 0.16 * vnoise(p * 10.0);
            a = mix(vec3<f32>(tint * grain), vec3<f32>(0.66, 0.64, 0.6), mortar);
            r = 0.1 * mortar;
        }
        case 8: { // Ice: faint cracks and cloudiness
            let s = fbm(p * 0.8);
            let crack = 1.0 - smoothstep(0.0, 0.025, abs(s - 0.5));
            a = vec3<f32>(0.94 + 0.1 * s + 0.12 * crack);
            r = 0.12 * crack;
        }
        case 9: { // Sand: grains and soft ripples
            let grains = fine(vnoise(p * 16.0), 16.0, px);
            let ripple = vnoise(vec3<f32>(q.x * 1.3, q.y * 0.45, 4.0));
            a = vec3<f32>(0.88 + 0.12 * grains + 0.1 * ripple);
            r = 0.03 * (grains - 0.5);
        }
        default: {}
    }
    return vec4<f32>(a, r);
}

// ---- shadows -----------------------------------------------------------

// Shared by production shading and the numerical GPU regression probe.
fn fragment_view_depth(in: VOut) -> f32 {
    // Baseline checkpoint retains the original calculation for executed red evidence.
    return 1.0 / in.clip.w;
}

// Returns the cascade count when the surface is beyond every cascade.
fn shadow_cascade(view_depth: f32) -> u32 {
    let count = u32(g.params.x);
    for (var i = 0u; i < count; i = i + 1u) {
        if (view_depth < g.cascade_splits[i]) {
            return i;
        }
    }
    return count;
}

fn shadow_factor(wp: vec3<f32>, n: vec3<f32>, view_depth: f32, frag: vec2<f32>) -> f32 {
    let count = i32(g.params.x);
    if (g.light_flags.x < 0.5) {
        return 1.0;
    }
    let ci = shadow_cascade(view_depth);
    if (ci >= u32(count)) {
        return 1.0;
    }
    let texel = g.cascade_texel[ci];
    let p = wp + n * texel * 2.5;
    let clip = g.cascade_vp[ci] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let ref_depth = clip.z - 0.0004;
    let lit = shadow_filter(uv, ci, ref_depth, frag);
    let last = g.cascade_splits[count - 1];
    let fade = smoothstep(0.85, 1.0, view_depth / last);
    return mix(lit, 1.0, fade);
}

// ---- shading models ----------------------------------------------------

// Wrapped Lambert normalised so the integral over the sphere matches Lambert (`diffuse_ndl`
// with an explicit wrap).
fn wrap_ndl(ndl_raw: f32, w: f32) -> f32 {
    return max((ndl_raw + w) / ((1.0 + w) * (1.0 + w)), 0.0);
}

// The finish pass's exposure on the scene image (fixed and automatic), as `unfinish` reads it.
fn scene_exposure() -> f32 {
    var m = g.light_toon.y;
    if (!(m > 0.0 && m < 1e30)) {
        m = 1.0;
    }
    return max(g.params.z * m, 1e-6);
}

// Lit: GGX specular and wrapped Lambert diffuse with sky light and reflections.
fn shade_lit(albedo: vec3<f32>, rough: f32, metal: f32, wrap: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, sh: f32) -> vec3<f32> {
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let diffuse_col = albedo * (1.0 - metal);
    let ndl = max(dot(n, l), 0.0);
    let ndv = max(dot(n, v), 1e-4);
    let h = normalize(l + v);
    let ndh = max(dot(n, h), 0.0);
    let vdh = max(dot(v, h), 0.0);
    let a = rough * rough;
    let a2 = a * a;
    let dd = ndh * ndh * (a2 - 1.0) + 1.0;
    let big_d = a2 / (PI * dd * dd);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let big_g = (ndl / (ndl * (1.0 - k) + k)) * (ndv / (ndv * (1.0 - k) + k));
    let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
    let spec = big_d * big_g * fres / max(4.0 * ndl * ndv, 1e-3);
    // The Look's wrap and LightingStyle.Soft's: the larger one bends the terminator.
    let w = max(wrap, g.light_flags.y);
    let direct = (diffuse_col / PI * wrap_ndl(dot(n, l), w) + spec * ndl) * g.sun_color.rgb * sh;
    return direct + ambient_light(diffuse_col, f0, n, v, rough, ndv, sh);
}

// Flat: Lit's diffuse and sky light on the triangle's own normal, without the highlight, so
// every face reads as one tone.
fn shade_flat(albedo: vec3<f32>, rough: f32, metal: f32, wrap: f32, nf: vec3<f32>, v: vec3<f32>, l: vec3<f32>, sh: f32) -> vec3<f32> {
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let diffuse_col = albedo * (1.0 - metal);
    let ndv = max(dot(nf, v), 1e-4);
    let w = max(wrap, g.light_flags.y);
    let direct = diffuse_col / PI * wrap_ndl(dot(nf, l), w) * g.sun_color.rgb * sh;
    return direct + ambient_light(diffuse_col, f0, nf, v, max(rough, 0.6), ndv, sh);
}

// Toon light steps: `bands` lit steps over the light amount x (0..1), edges `width` wide in
// step units. 1 band is a two-tone lit/shade split at the terminator.
fn toon_steps(x: f32, bands: f32, width: f32) -> f32 {
    if (bands <= 1.0) {
        return smoothstep(0.02 - width, 0.02 + width, x);
    }
    let s = x * bands;
    let base = floor(s);
    let edge = smoothstep(0.5 - width * bands, 0.5 + width * bands, s - base);
    return clamp((base + edge) / bands, 0.0, 1.0);
}

// Light arriving at a surface quantised to steps of `ev` EV around mid-grey as displayed, hue
// kept (the Pixel 3D palette then survives exposure changes).
fn quantise_light(light: vec3<f32>, ev: f32) -> vec3<f32> {
    let e = scene_exposure();
    let y = max(dot(light, vec3<f32>(0.2126, 0.7152, 0.0722)) * e, 1e-6);
    let q = floor(log2(y / 0.18) / ev + 0.5) * ev;
    return light * (0.18 * exp2(q) / y);
}

fn unpack_rgb8(p: f32) -> vec3<f32> {
    let u = u32(p + 0.5);
    let s = vec3<f32>(f32((u >> 16u) & 255u), f32((u >> 8u) & 255u), f32(u & 255u)) / 255.0;
    // sRGB to linear (the same curve as `srgb_to_linear` in scene.rs).
    return select(pow((s + 0.055) / 1.055, vec3<f32>(2.4)), s / 12.92, s <= vec3<f32>(0.04045));
}

// Toon: the sun term (N.L times the cast shadow) cut into bands, the shade side tinted, ambient
// flattened toward the sky and ground average, a highlight blob and an optional rim.
// `step_aa`, `spec_aa` and `rim_aa` are derivative widths taken in uniform control flow.
fn shade_toon(albedo: vec3<f32>, row: MaterialRow, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, sh: f32, step_aa: f32, spec_aa: f32, rim_aa: f32) -> vec3<f32> {
    let lit_x = clamp(dot(n, l), 0.0, 1.0) * sh;
    let sun_term = toon_steps(lit_x, row.toon.x, max(row.toon.y, step_aa));
    let up = sky_hemisphere(vec3<f32>(0.0, 1.0, 0.0));
    let down = sky_hemisphere(vec3<f32>(0.0, -1.0, 0.0));
    let hemi = mix(sky_hemisphere(n), 0.5 * (up + down), row.toon.z);
    let open = mix(0.5 + 0.5 * n.y, 0.5, row.toon.z) * mix(0.55, 1.0, sh);
    let ambient = hemi * g.light_ambient.w + g.light_ambient.rgb + g.light_outdoor.rgb * open;
    let sun = g.sun_color.rgb / PI;
    var light = sun * sun_term + ambient * mix(row.tint.rgb, vec3<f32>(1.0), sun_term);
    if (row.toon.w > 0.0) {
        light = quantise_light(light, row.toon.w);
    }
    var col = albedo * light;
    // Highlight blob: a hard-edged Blinn-Phong spot of size `tint.w` on the lit side.
    if (row.tint.w > 0.0) {
        let ndh = dot(n, normalize(l + v));
        let t = 1.0 - row.tint.w;
        col = col + sun * (row.extra.x * smoothstep(t - spec_aa, t + spec_aa, ndh) * sun_term);
    }
    // Rim on the lit side.
    if (row.extra.y > 0.0) {
        let rt = 1.0 - row.extra.y;
        let rim = smoothstep(rt - rim_aa, rt + rim_aa, 1.0 - max(dot(n, v), 0.0)) * sun_term;
        col = col + unpack_rgb8(row.extra.z) * sun * (0.4 * rim);
    }
    return col;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let row = material_row(in.material.x);
    let model = u32(row.surface.x + 0.5);
    let n = normalize(in.world_n);
    let to_cam = g.camera_pos.xyz - in.world_pos;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-4);
    let l = g.sun_dir.xyz;

    // Every derivative is taken here, in uniform control flow, not inside a model's branch.
    let fw = fwidth(in.local_p);
    let px = max(fw.x, max(fw.y, fw.z));
    // Flat: the triangle's normal from the screen derivatives, turned to the shading normal's side.
    let face = cross(dpdx(in.world_pos), dpdy(in.world_pos));
    let face_len2 = dot(face, face);
    var nf = select(n, face * inverseSqrt(max(face_len2, 1e-30)), face_len2 > 1e-24);
    nf = select(nf, -nf, dot(nf, n) < 0.0);
    // Toon edge widths: about one pixel of the light amount, the half-vector and the rim term.
    let step_aa = 0.5 * fwidth(clamp(dot(n, l), 0.0, 1.0));
    let spec_aa = max(fwidth(dot(n, normalize(l + v))), 1e-4);
    let rim_aa = max(fwidth(dot(n, v)), 1e-4);

    let d = detail(i32(in.material.z), in.local_p, in.local_n, px);
    let albedo = in.color.rgb * d.rgb;
    // The row's floor holds for skinned slots too (parts' rows are floored already).
    let rough = clamp(max(in.surf.x, row.extra.w) + d.w, 0.04, 1.0);
    let metal = in.surf.y;

    if (model == MODEL_UNLIT) {
        // Shown as its own colour: through `unfinish`, like toon characters (the detail
        // multiplier is applied after it). Alpha 0 on opaque pixels keeps them out of the
        // exposure meter (their light follows the exposure).
        let a = select(in.color.a, 0.0, in.color.a >= 0.999);
        return vec4<f32>(apply_fog(in.unlit * d.rgb, in.world_pos), a);
    }
    var sn = n;
    if (model == MODEL_FLAT) {
        sn = nf;
    }
    let view_depth = fragment_view_depth(in);
    let sh = shadow_factor(in.world_pos, sn, view_depth, in.clip.xy);
    var col: vec3<f32>;
    switch model {
        case MODEL_TOON: {
            col = shade_toon(albedo, row, n, v, l, sh, step_aa, spec_aa, rim_aa);
        }
        case MODEL_FLAT: {
            col = shade_flat(albedo, rough, metal, row.surface.w, nf, v, l, sh);
        }
        default: {
            col = shade_lit(albedo, rough, metal, row.surface.w, n, v, l, sh);
        }
    }
    col = apply_fog(col, in.world_pos);
    return vec4<f32>(col, in.color.a);
}
