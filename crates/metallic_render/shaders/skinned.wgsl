// Skinned meshes in the main pass. Writes the same VOut as vs_main, so fs_main shades skinned
// characters exactly like parts.
@vertex
fn vs_skinned(v: SkinIn) -> VOut {
    let m = skin_matrix(v.joints, v.weights);
    let wp = m * vec4<f32>(v.pos, 1.0);
    let s = min(v.slot, 7u);
    var out: VOut;
    out.clip = g.view_proj * wp;
    out.world_pos = wp.xyz;
    // Joint matrices carry rotation and uniform scale only, so the normal needs no inverse.
    out.world_n = normalize((m * vec4<f32>(v.normal, 0.0)).xyz);
    // Procedural detail sticks to the bind pose, so it does not swim as the joints move.
    out.local_p = v.pos * skin.params.x;
    out.local_n = v.normal;
    out.color = skin.slot_color[s];
    out.material = skin.slot_material[s];
    return out;
}

// ---- Toon style: warm two-tone shading and an ink outline (`SkinStyle::Toon`).

// Display-linear tones relative to the slot colour: lit is the colour itself, the shade is warmer
// and darker, the core (facing away from the sun) a little darker again. Picked against the
// concept sheet (lit #e9d8b6, shade about #c1a380, core about #a78b6d).
const TOON_SHADE: vec3<f32> = vec3<f32>(0.66, 0.54, 0.46);
const TOON_CORE: vec3<f32> = vec3<f32>(0.50, 0.39, 0.33);
// Ink colour, display-linear (sRGB #5b3b33).
const TOON_INK: vec3<f32> = vec3<f32>(0.1046, 0.0437, 0.0331);

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// How bright the scene's light is next to the default sky (1 under the default sun), with a
// little of its hue, so toon characters still dim at dusk and pick up a coloured sun.
fn toon_light() -> vec3<f32> {
    let sky = mix(g.sky_horizon.rgb, g.sky_zenith.rgb, 0.45);
    let e = g.sun_color.rgb * 0.27 + sky * 0.3;
    let k = luma(e) / 0.95;
    return clamp(mix(vec3<f32>(k), e / 0.95, 0.25), vec3<f32>(0.0), vec3<f32>(1.25));
}

fn toon_fog(c: vec3<f32>, wp: vec3<f32>) -> vec3<f32> {
    let dist = length(g.camera_pos.xyz - wp);
    let fog = 1.0 - exp(-pow(dist * g.camera_pos.w, 2.0));
    return mix(c, g.sky_horizon.rgb, clamp(fog, 0.0, 1.0));
}

@fragment
fn fs_toon(in: VOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_n);
    let v = normalize(g.camera_pos.xyz - in.world_pos);
    let ndl = dot(n, g.sun_dir.xyz);
    let sh = shadow_factor(in.world_pos, n, 1.0 / in.clip.w);
    // Soft band edges: a few hundredths of the light range, about a pixel or two wide. The
    // band sits a little past the true terminator so planes turned partly away read as shade.
    // The cast-shadow term stays as soft as the filtered shadow map: a hard step on it shows the
    // shadow map's texels as a serrated edge (the head's shadow on the chest).
    let lit = smoothstep(0.12, 0.20, ndl) * sh;
    let front = smoothstep(-0.45, -0.35, ndl);
    let base = in.color.rgb;
    var tone = mix(base * TOON_CORE, base * TOON_SHADE, front);
    tone = mix(tone, base, lit);
    // A thin, slightly darker warm line just inside the lit band (the terminator).
    let term = smoothstep(0.12, 0.20, ndl) * (1.0 - smoothstep(0.20, 0.30, ndl));
    tone = tone * (1.0 - 0.07 * term);
    // A faint rim of reflected light on the shade side, so the form separates from the ink.
    let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * (1.0 - lit);
    tone = mix(tone, base * TOON_SHADE * 1.18, 0.5 * rim);
    let c = unfinish(tone * toon_light());
    return vec4<f32>(toon_fog(c, in.world_pos), 1.0);
}

struct OutlineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
}

// Inverted hull: the back faces pushed out along the smoothed normal by a fixed number of pixels,
// so the line keeps its width on screen. It thins with distance once the figure is small (below
// about 350 px tall) so far crowds do not turn into ink.
@vertex
fn vs_outline(v: SkinIn) -> OutlineOut {
    let m = skin_matrix(v.joints, v.weights);
    let wp = m * vec4<f32>(v.pos, 1.0);
    let wn = normalize((m * vec4<f32>(v.smooth_n, 0.0)).xyz);
    // 0.02 studs further from the camera than the surface, so where parts meet (the neck into the
    // shoulders) the hull loses depth ties to the surface instead of showing through as dashes.
    let away = normalize(wp.xyz - g.camera_pos.xyz) * 0.02;
    var clip = g.view_proj * vec4<f32>(wp.xyz + away, 1.0);
    let cn = (g.view_proj * vec4<f32>(wn, 0.0)).xy;
    let len = length(cn);
    if (len > 1e-6) {
        // Pixels per stud at this depth. Row 1 of view_proj is the projection's vertical scale
        // times a unit row of the view rotation, so its length is that scale.
        let sy = length(vec3<f32>(g.view_proj[0][1], g.view_proj[1][1], g.view_proj[2][1]));
        let px_per_stud = sy * 0.5 * g.viewport.y / max(clip.w, 1e-4);
        let px = min(skin.params.z, skin.params.w * px_per_stud);
        clip = vec4<f32>(clip.xy + cn / len * px * 2.0 * g.viewport.zw * clip.w, clip.zw);
    }
    var out: OutlineOut;
    out.clip = clip;
    out.world_pos = wp.xyz;
    return out;
}

@fragment
fn fs_outline(in: OutlineOut) -> @location(0) vec4<f32> {
    let c = unfinish(TOON_INK * min(toon_light(), vec3<f32>(1.0)));
    return vec4<f32>(toon_fog(c, in.world_pos), 1.0);
}
