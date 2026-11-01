// Metallic VFX sample: draws the particles simulated by particles_sim.wgsl as camera-facing quads.
//
// Research sample, not engine code. Drawn with draw_indirect from the args cs_finish wrote, so the CPU never
// knows the particle count. Features: flipbook with frame blending, soft particles (fade near scene depth),
// premultiplied alpha output so additive and alpha-blended particles share one blend state
// (src = One, dst = OneMinusSrcAlpha; additive = rgb with alpha 0).
// Validate: naga docs/research/style-shaders/particles_draw.wgsl

struct Particle {
    position: vec3<f32>,
    age: f32,
    velocity: vec3<f32>,
    lifetime: f32,
    color: vec4<f32>,
    size: f32,
    rotation: f32,
    spin: f32,
    seed: f32,
}

struct Frame {
    view_proj: mat4x4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    near_far: vec2<f32>,
    viewport: vec2<f32>,
}

struct Look {
    flipbook: vec2<f32>,    // columns, rows (1, 1 = single image)
    fps: f32,               // 0 = play once over the particle's life (age), > 0 = loop at this rate
    soft_distance: f32,     // fade distance in world units, 0 disables soft particles
    additive: f32,          // 0 = alpha blend, 1 = additive
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> alive: array<u32>; // the live half of particles_sim's `lists`
@group(0) @binding(3) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var<uniform> look: Look;
@group(1) @binding(1) var atlas: texture_2d<f32>;
@group(1) @binding(2) var atlas_samp: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv_a: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) blend: f32,
    @location(3) color: vec4<f32>,
    @location(4) view_depth: f32,
}

fn frame_uv(local: vec2<f32>, index: f32) -> vec2<f32> {
    let cols = max(look.flipbook.x, 1.0);
    let rows = max(look.flipbook.y, 1.0);
    let total = cols * rows;
    let i = index - total * floor(index / total);
    let cell = vec2<f32>(i - cols * floor(i / cols), floor(i / cols));
    return (cell + local) / vec2<f32>(cols, rows);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let p = particles[alive[ii]];
    // Two triangles: corners in -0.5..0.5.
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5), vec2<f32>(-0.5, 0.5),
    );
    let c = corners[vi];
    let cs = cos(p.rotation);
    let sn = sin(p.rotation);
    let r = vec2<f32>(c.x * cs - c.y * sn, c.x * sn + c.y * cs) * p.size;
    let world = p.position + frame.camera_right.xyz * r.x + frame.camera_up.xyz * r.y;

    let t = p.age / max(p.lifetime, 1e-4);
    let total = max(look.flipbook.x * look.flipbook.y, 1.0);
    // With fps 0 the flipbook plays once over the particle's life: frame 0 at birth, the last frame at
    // death, never blending back into frame 0. With fps > 0 it loops, so the wrap is intended.
    let by_age = look.fps <= 0.0;
    let f = select(p.age * look.fps, clamp(t, 0.0, 1.0) * (total - 1.0), by_age);
    let next = select(floor(f) + 1.0, min(floor(f) + 1.0, total - 1.0), by_age);
    let local = vec2<f32>(c.x + 0.5, 0.5 - c.y);

    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv_a = frame_uv(local, floor(f));
    out.uv_b = frame_uv(local, next);
    out.blend = f - floor(f);
    out.color = p.color;
    out.view_depth = out.clip.w;
    return out;
}

fn linear_depth(d: f32) -> f32 {
    let n = frame.near_far.x;
    let f = frame.near_far.y;
    return n * f / (f - d * (f - n));
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let a = textureSample(atlas, atlas_samp, in.uv_a);
    let b = textureSample(atlas, atlas_samp, in.uv_b);
    var tex = mix(a, b, in.blend) * in.color;
    if (look.soft_distance > 0.0) {
        let scene = linear_depth(textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0));
        tex.a = tex.a * clamp((scene - in.view_depth) / look.soft_distance, 0.0, 1.0);
    }
    let premul = tex.rgb * tex.a;
    return vec4<f32>(premul, tex.a * (1.0 - look.additive));
}
