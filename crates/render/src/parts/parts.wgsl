// Parts: instanced meshes (the primitives and mesh assets), shaded PBR-lite (Lambert diffuse, GGX specular) under one directional light
// (the sun, or the moon at night) and a sky/ground hemisphere ambient, all from `Lighting`, plus the local point and
// spot lights (lights.wgsl). One shading model;
// the material row picks the parameters. frame.wgsl comes before this
// file, lights.wgsl after it, and the vertex entry point last, from instance_storage.wgsl or instance_vertex.wgsl
// (scene/gpu.rs explains the two paths).
//
// Positions are camera-relative: instance translations are relative to the scene origin, and `frame.camera_offset`
// (camera − origin) is subtracted before the rotation-only view-projection, so f32 stays precise far from the world
// origin (ADR 0008 §11).

const PI: f32 = 3.14159265;

// `MaterialRow` in parts/material.rs.
struct MaterialRow {
    // x: roughness, y: metalness, z: emissive, w: 1 for glass.
    surface: vec4<f32>,
    reserved: vec4<f32>,
}

// MAX_MATERIAL_ROWS in parts/material.rs.
const MATERIAL_ROWS: u32 = 256u;

@group(0) @binding(1) var<uniform> materials: array<MaterialRow, 256>;

// The sun's shadow cascades, one layer each, and a comparison sampler that filters four depth tests bilinearly.
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
// What is above each point (#615): the sky map, a one-cascade shadow map lit from straight overhead.
@group(0) @binding(13) var sky_map: texture_depth_2d_array;
// The cascades' see-through maps (#491): the nearest see-through casters' depth and the light they let through.
@group(0) @binding(8) var see_through_depth: texture_depth_2d_array;
@group(0) @binding(9) var see_through_light: texture_2d_array<f32>;
@group(0) @binding(10) var see_through_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Camera-relative world position.
    @location(0) position: vec3<f32>,
    // World normal, not normalised.
    @location(1) normal: vec3<f32>,
    // Linear colour; alpha is 1 − Transparency.
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) material: u32,
    // Where the base-colour texture is sampled (`base_texture`, bound by the instance file: a mesh asset's texture,
    // or white for every other draw).
    @location(4) uv: vec2<f32>,
}

// The packed sRGB colour of an instance (red in the lowest byte), as linear light.
fn unpack_color(packed: u32) -> vec3<f32> {
    let srgb = vec3<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u), f32((packed >> 16u) & 255u)) / 255.0;
    return srgb_to_linear(srgb);
}

// The level of detail of vertex number `vertex` in the mesh pool, for the debug view of the levels (lod.rs,
// `Renderer::set_lod_tint`): `frame.output.yzw` hold where levels 1, 2 and 3 start in the pool, which lays the meshes
// out level by level (0 when the view is off).
fn pool_level(vertex: u32) -> u32 {
    let v = f32(vertex);
    return u32(v >= frame.output.y) + u32(v >= frame.output.z) + u32(v >= frame.output.w);
}

// The debug view of the levels of detail, when it is on (`frame.output.y` above 0): the round kinds (Ball 1,
// Cylinder 2: the ones with levels) take their level's colour, from green at full detail to red at the coarsest, and
// every other kind is grey.
fn lod_tint(level: u32, mesh_material: u32, color: vec3<f32>) -> vec3<f32> {
    if frame.output.y <= 0.0 {
        return color;
    }
    let kind = mesh_material >> 16u;
    if kind != 1u && kind != 2u {
        return vec3<f32>(0.3);
    }
    var tints = array<vec3<f32>, 4>(
        vec3<f32>(0.05, 0.6, 0.05),
        vec3<f32>(0.8, 0.7, 0.02),
        vec3<f32>(0.9, 0.25, 0.01),
        vec3<f32>(0.7, 0.01, 0.01),
    );
    return tints[min(level, 3u)];
}

// Places a mesh vertex (of a mesh at level of detail `level`) with an instance's 3x4 affine (rows r0..r2) and packs
// what the fragment needs.
fn place(
    level: u32,
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    color: u32,
    transparency: f32,
    mesh_material: u32,
) -> VertexOut {
    let p = vec4<f32>(position, 1.0);
    let world = vec3<f32>(dot(r0, p), dot(r1, p), dot(r2, p)) - frame.camera_offset.xyz;
    // The cofactor matrix is the inverse transpose up to a positive scale: correct under non-uniform Size.
    let c0 = vec3<f32>(r0.x, r1.x, r2.x);
    let c1 = vec3<f32>(r0.y, r1.y, r2.y);
    let c2 = vec3<f32>(r0.z, r1.z, r2.z);
    let cofactor = mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1));
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.position = world;
    out.normal = cofactor * normal;
    out.color = vec4<f32>(lod_tint(level, mesh_material, unpack_color(color)), 1.0 - transparency);
    out.material = mesh_material & 0xffffu;
    out.uv = uv;
    return out;
}

// The base colour at a fragment: the instance's colour times its texture's (linear, since the texture is sRGB).
fn albedo_of(in: VertexOut) -> vec3<f32> {
    return in.color.rgb * textureSample(base_texture, base_sampler, in.uv).rgb;
}

fn material_row(row: u32) -> MaterialRow {
    return materials[select(0u, row, row < MATERIAL_ROWS)];
}

// Split-sum environment BRDF, analytic fit (Karis 2014): how much of the ambient light a surface reflects.
fn env_brdf(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// How much of the sun (or moon) reaches a point: 1 lit, 0 in shadow. `position` is camera-relative, `n` the unit
// normal and `n_dot_l` its cosine to the light. The cascade is the first whose slice of the view the point is in;
// the point moves along its normal by the cascade's offset, scaled by 1 − N·L (only grazing receivers need it), and
// is compared with its bias, through a tent of bilinear comparison taps (`shadow_params.y` texels each way). Past
// the last cascade there is no shadow, faded in over the last tenth.
//
// The cascade is counted without a loop: the splits grow, so it is how many of the cascades in use end at or before
// the point. With a search loop, which neighbouring pixels on either side of a split leave at different steps,
// lavapipe (llvmpipe, LLVM 21) ran the filter loop after it zero times for a few pixels just before the first split
// (#149): no tap, a total weight of 0, and 0 / 0. The total is also the tent's, (reach + 1)⁴, rather than a sum in
// the loop, so it is never 0 whatever a driver does with the loop.
fn sun_visibility(position: vec3<f32>, n: vec3<f32>, n_dot_l: f32) -> f32 {
    let count = u32(frame.shadow_params.x);
    if count == 0u {
        return 1.0;
    }
    let depth = dot(position, frame.view_forward.xyz);
    let ended = (vec4<f32>(depth) >= frame.shadow_splits) & (vec4<u32>(0u, 1u, 2u, 3u) < vec4<u32>(count));
    let cascade = u32(ended.x) + u32(ended.y) + u32(ended.z) + u32(ended.w);
    if cascade >= count {
        return 1.0;
    }
    let p = position + n * (frame.shadow_offsets[cascade] * (1.0 - n_dot_l));
    let clip = frame.shadow_matrices[cascade] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let reference = clip.z - frame.shadow_biases[cascade];
    // Lighting.ShadowSoftness spreads the taps (#600): 1 at its default.
    let texel = frame.shadow_params.z * frame.environment.z;
    let reach = i32(frame.shadow_params.y);
    var lit = 0.0;
    for (var y = -reach; y <= reach; y += 1) {
        for (var x = -reach; x <= reach; x += 1) {
            let weight = f32(reach + 1 - abs(x)) * f32(reach + 1 - abs(y));
            let at = uv + vec2<f32>(f32(x), f32(y)) * texel;
            lit += weight * textureSampleCompareLevel(shadow_maps, shadow_sampler, at, cascade, reference);
        }
    }
    let side = f32(max(reach, 0) + 1);
    let total = side * side * side * side;
    var visible = lit / total;
    // Behind a see-through caster, as much light as it lets through (#491): its light map filtered, where the point is
    // farther from the light than the nearest see-through caster (the part itself and what is in front of it are
    // not dimmed). The maps are half the cascade's resolution, so their bias is twice as large.
    if frame.shadow_see_through[cascade] > 0.5 {
        let front = textureSampleCompareLevel(
            see_through_depth,
            shadow_sampler,
            uv,
            cascade,
            reference - frame.shadow_biases[cascade],
        );
        let through = textureSampleLevel(see_through_light, see_through_sampler, uv, cascade, 0.0).r;
        visible *= mix(through, 1.0, front);
    }
    let fade = smoothstep(frame.shadow_params.w, frame.shadow_splits[count - 1u], depth);
    return mix(visible, 1.0, fade);
}

// The share of a box of (2 × `reach` + 1)² taps, `step` texels apart, around `p` (camera-relative) that has nothing above
// it in the sky map: 0 to 1. Every tap weighs the same: a tent changes fastest at its middle, just where the shaping
// in `sky_open` starts, and the edge would start with a step.
fn sky_share(p: vec3<f32>, reach: i32, step: f32) -> f32 {
    let clip = frame.sky_matrix * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let reference = clip.z - frame.sky_params.w;
    let spread = frame.sky_params.y * step;
    var open = 0.0;
    for (var y = -reach; y <= reach; y += 1) {
        for (var x = -reach; x <= reach; x += 1) {
            let at = uv + vec2<f32>(f32(x), f32(y)) * spread;
            open += textureSampleCompareLevel(sky_map, shadow_sampler, at, 0, reference);
        }
    }
    let side = f32(2 * reach + 1);
    return open / (side * side);
}

// How open a point is to the sky's light (#615), 0 to 1: Lighting.OutdoorAmbient lights only that much of it. The sky
// map holds the nearest surface above each point; a point under something is closed. It is only a height map, so it
// cannot tell a wall from an open side, nor a ball's underside from a ceiling: both have their own part above them.
// So the two halves of the sky's light are kept apart (#648):
//
// - x, what comes down from the sky, onto what faces up and sideways: open where the point itself is, and closed under
//   anything. A narrow box of 3×3 taps, of which half open is still closed (#642), so it never reaches through a wall
//   into a room, and darkens open ground beside a part over about a stud only (#648; four studs before). The point
//   moves out along its normal first (a wall's outside is open, its inside under a roof is not); a wall's, by the
//   filter's reach as well, so the wall's own top, beside it in the map, does not close it. Never down: under a ball,
//   the floor would close its underside in a hard line (#642).
// - y, what the ground throws back up, onto what faces down: open where the ground around the point is, a box of 5×5
//   taps two texels apart, from the point moved out sideways by as much. A ball's underside sees the open ground round
//   it; a ceiling sees the room's covered floor, and stays dark however near a wall, as more than half of what it sees
//   is still covered.
//
// Past the map, everything is open, faded in over its last tenth.
fn sky_open(position: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    if frame.sky_params.x < 0.5 {
        return vec2<f32>(1.0);
    }
    let depth = dot(position, frame.view_forward.xyz);
    if depth >= frame.sky_range.x {
        return vec2<f32>(1.0);
    }
    let lift = frame.sky_params.z;
    let out = vec3<f32>(n.x, max(n.y, 0.0), n.z);
    let above = position + out * (lift + frame.sky_range.z * length(n.xz));
    let sky = smoothstep(0.5, 1.0, sky_share(above, 1, 1.0));
    let around = position + vec3<f32>(n.x, 0.0, n.z) * (frame.sky_range.w * GROUND_REACH) + vec3<f32>(0.0, lift, 0.0);
    let ground = smoothstep(0.6, 1.0, sky_share(around, 2, 2.0));
    let fade = smoothstep(frame.sky_range.y, frame.sky_range.x, depth);
    return mix(vec2<f32>(sky, ground), vec2<f32>(1.0), fade);
}

// How far the ground's taps reach from a point, in texels: two taps two texels apart, and the compare's half texel.
const GROUND_REACH: f32 = 4.5;

// The sky's light on a surface facing `n` (or, for its reflections, looking along `n`): the sky's half where the sky
// reaches, the ground's where the ground around is open (`open`, from `sky_open`).
fn sky_light(n_y: f32, open: vec2<f32>) -> vec3<f32> {
    let up = clamp(n_y * 0.5 + 0.5, 0.0, 1.0);
    return frame.sky_ambient.rgb * (up * open.x) + frame.ground_ambient.rgb * ((1.0 - up) * open.y);
}

// `v` scaled to unit length, or 0 when it has none (where `normalize` divides 0 by 0).
fn unit_or_zero(v: vec3<f32>) -> vec3<f32> {
    return v * inverseSqrt(max(dot(v, v), 1e-30));
}

struct Shaded {
    // Diffuse, ambient and emitted light: what transparency fades.
    body: vec3<f32>,
    // Specular reflection (direct and ambient).
    specular: vec3<f32>,
}

fn shade(in: VertexOut, albedo: vec3<f32>) -> Shaded {
    let row = material_row(in.material);
    let roughness = clamp(row.surface.x, 0.045, 1.0);
    let metalness = row.surface.y;
    let n = normalize(in.normal);
    let v = normalize(-in.position);
    let l = frame.light_direction.xyz;

    let f0 = mix(vec3<f32>(0.04), albedo, metalness);
    let diffuse = albedo * (1.0 - metalness);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    // l + v is 0 where the view ray points straight at the light: a surface seen against the sun's disc. The cosines
    // are clamped to 1 as well as 0: rounding takes them a little past 1 where h meets n or v, and pow(1 − v·h, 5)
    // is not a number for a negative base on drivers that take it as exp2(5 × log2(1 − v·h)).
    let h = unit_or_zero(l + v);
    let n_dot_h = clamp(dot(n, h), 0.0, 1.0);
    let v_dot_h = clamp(dot(v, h), 0.0, 1.0);

    // GGX distribution, height-correlated Smith visibility, Schlick Fresnel.
    let a = roughness * roughness;
    let a2 = a * a;
    let d_denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * d_denominator * d_denominator);
    let visibility = 0.5 / max(
        n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2) + n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2),
        1e-5,
    );
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
    var shadow = 1.0;
    if n_dot_l > 0.0 {
        shadow = sun_visibility(in.position, n, n_dot_l);
    }
    let light = frame.light_color.rgb * n_dot_l * shadow;

    var out: Shaded;
    // Lighting.EnvironmentDiffuseScale and EnvironmentSpecularScale scale the sky's light, ColorShift_Bottom lights
    // the side away from the sun (#600).
    let away = max(-dot(n, l), 0.0);
    // The sky's light only where the sky reaches; Ambient everywhere (#615).
    let open = sky_open(in.position, n);
    out.body = diffuse / PI * (light + frame.color_shift_bottom.rgb * away)
        + diffuse * (sky_light(n.y, open) * frame.environment.x + frame.ambient.rgb)
        + albedo * row.surface.z;
    out.specular = distribution * visibility * fresnel * light
        + env_brdf(f0, roughness, n_dot_v) * (sky_light(reflect(-v, n).y, open) + frame.ambient.rgb) * frame.environment.y;
    // Frames that draw no local light use pipelines made with lights_off.wgsl, whose `local_lighting` adds nothing
    // and binds nothing (parts/mod.rs).
    let local = local_lighting(in.clip.xy, in.position, n, v, n_dot_v, diffuse, f0, a2);
    out.body += local.body;
    out.specular += local.specular;
    return out;
}

// The light-density view's colour for `count` lights (#478): dark blue for none, then blue, cyan, green, yellow,
// orange, and red for 8 or more.
fn density_heat(count: u32) -> vec3<f32> {
    var ramp = array<vec3<f32>, 7>(
        vec3<f32>(0.01, 0.01, 0.06),
        vec3<f32>(0.02, 0.08, 0.6),
        vec3<f32>(0.0, 0.5, 0.6),
        vec3<f32>(0.05, 0.6, 0.05),
        vec3<f32>(0.8, 0.7, 0.02),
        vec3<f32>(0.9, 0.3, 0.01),
        vec3<f32>(0.8, 0.01, 0.01),
    );
    let at = min(count, 8u);
    // 5 to 7 lights stay orange; 8 or more are red.
    return ramp[select(min(at, 5u), 6u, at >= 8u)];
}

// What a shading view other than Shaded draws at a fragment (parts/shading.rs, #478), before transparency: the
// part's own colour (unlit), the light alone on a grey surface, or the light-density heat map over the surface's
// sunlit shape. `albedo` is sampled by the caller, in uniform control flow.
fn view_color(in: VertexOut, albedo: vec3<f32>, view: u32) -> vec3<f32> {
    if view == 1u {
        return albedo;
    }
    if view == 2u {
        let s = shade(in, vec3<f32>(0.5));
        return s.body + s.specular;
    }
    let n = normalize(in.normal);
    let count = local_light_count(in.clip.xy, in.position);
    let sunlit = 0.45 + 0.55 * max(dot(n, frame.light_direction.xyz), 0.0);
    return density_heat(count) * sunlit;
}

@fragment
fn fs_opaque(in: VertexOut) -> @location(0) vec4<f32> {
    // Sampled first, in uniform control flow.
    let albedo = albedo_of(in);
    let view = u32(frame.view.x);
    if view != 0u {
        return vec4<f32>(encode(view_color(in, albedo, view)), 1.0);
    }
    let s = shade(in, albedo);
    return vec4<f32>(encode(finish(s.body + s.specular, length(in.position))), 1.0);
}

// Premultiplied alpha. Glass keeps its reflections at full strength while its body fades.
@fragment
fn fs_transparent(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = albedo_of(in);
    let alpha = in.color.a;
    let view = u32(frame.view.x);
    if view != 0u {
        return vec4<f32>(encode(view_color(in, albedo, view) * alpha), alpha);
    }
    let s = shade(in, albedo);
    let glass = material_row(in.material).surface.w > 0.5;
    let specular = select(s.specular * alpha, s.specular, glass);
    // Through the fog (#600) the surface gives way to the fog's colour, as much as it covers.
    let lit = s.body * alpha + specular;
    let fogged = mix(lit, frame.fog_color.rgb * alpha, fog_amount(length(in.position)));
    return vec4<f32>(encode(fogged * frame.environment.w), alpha);
}

// The wireframe view's lines (#478): each part's colour, lifted a little so dark parts still show on the sky.
@fragment
fn fs_wireframe(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = albedo_of(in);
    return vec4<f32>(encode(mix(albedo, vec3<f32>(1.0), 0.3)), 1.0);
}
