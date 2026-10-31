// Physically based shading of instanced parts with procedural material detail.
struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) material: vec4<f32>,
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) local_p: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) material: vec4<f32>,
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
    out.world_pos = wp.xyz;
    out.world_n = normalize(cof * v.normal);
    out.local_p = v.pos * vec3<f32>(length(c0), length(c1), length(c2));
    out.local_n = v.normal;
    out.color = v.color;
    out.material = v.material;
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

fn shadow_factor(wp: vec3<f32>, n: vec3<f32>, view_depth: f32, frag: vec2<f32>) -> f32 {
    let count = i32(g.params.x);
    if (g.light_flags.x < 0.5) {
        return 1.0;
    }
    var ci = -1;
    for (var i = 0; i < count; i = i + 1) {
        if (view_depth < g.cascade_splits[i]) {
            ci = i;
            break;
        }
    }
    if (ci < 0) {
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

// ---- lighting ----------------------------------------------------------

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_n);
    let to_cam = g.camera_pos.xyz - in.world_pos;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-4);
    let l = g.sun_dir.xyz;

    // Derivatives must be taken here, in uniform control flow, not inside `detail`'s switch.
    let fw = fwidth(in.local_p);
    let px = max(fw.x, max(fw.y, fw.z));
    let d = detail(i32(in.material.z + 0.5), in.local_p, in.local_n, px);
    let albedo = in.color.rgb * d.rgb;
    let rough = clamp(in.material.x + d.w, 0.04, 1.0);
    let metal = in.material.y;
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

    let view_depth = 1.0 / in.clip.w;
    let sh = shadow_factor(in.world_pos, n, view_depth, in.clip.xy);
    let direct = (diffuse_col / PI * diffuse_ndl(dot(n, l)) + spec * ndl) * g.sun_color.rgb * sh;

    let ambient = ambient_light(diffuse_col, f0, n, v, rough, ndv, sh);

    var col = direct + ambient;
    col = apply_fog(col, in.world_pos);
    return vec4<f32>(col, in.color.a);
}
