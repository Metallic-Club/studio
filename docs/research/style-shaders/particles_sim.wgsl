// Metallic VFX sample: GPU particle emit + update in compute, with an indirect draw at the end.
//
// Research sample, not engine code. One emitter's pool of `capacity` particles lives in a storage buffer.
// Free slots sit on a dead list; live ones on an alive list that ping-pongs between two buffers each frame.
// Per frame, dispatched in order (one compute pass, wgpu inserts the barriers between dispatches):
//   cs_emit     (ceil(emit_count / 64) groups)   pops dead slots and initialises them, appends to alive_cur
//   cs_prepare  (1 group)                        writes the indirect dispatch size for cs_update
//   cs_update   (dispatch_workgroups_indirect)   ages, moves and kills particles; survivors go to alive_next
//   cs_finish   (1 group)                        writes DrawIndirect args for particles_draw.wgsl
// The CPU never reads counts back, so there is no stall. Bounds and sorting are separate passes.
// Validate: naga docs/research/style-shaders/particles_sim.wgsl

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

struct Counters {
    alive_cur: atomic<u32>,
    alive_next: atomic<u32>,
    dead: atomic<u32>,
    _pad: u32,
}

struct DrawIndirectArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

struct DispatchIndirectArgs {
    x: u32,
    y: u32,
    z: u32,
}

// Beginner-facing emitter properties map 1:1 onto this block (rate, lifetime, speed, spread, size, colour).
struct Emitter {
    origin: vec4<f32>,          // xyz: emitter position, w: spawn sphere radius
    direction: vec4<f32>,       // xyz: unit main direction, w: cone spread in radians
    speed_life: vec4<f32>,      // x: min speed, y: max speed, z: min lifetime, w: max lifetime
    gravity_drag: vec4<f32>,    // xyz: acceleration, w: linear drag per second
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    size_start_end: vec2<f32>,
    dt: f32,
    time: f32,
    emit_count: u32,
    capacity: u32,
    frame_seed: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> em: Emitter;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read_write> alive_cur: array<u32>;
@group(0) @binding(3) var<storage, read_write> alive_next: array<u32>;
@group(0) @binding(4) var<storage, read_write> dead_list: array<u32>;
@group(0) @binding(5) var<storage, read_write> counters: Counters;
@group(0) @binding(6) var<storage, read_write> draw_args: DrawIndirectArgs;
@group(0) @binding(7) var<storage, read_write> update_args: DispatchIndirectArgs;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand01(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967295.0;
}

fn random_in_cone(dir: vec3<f32>, spread: f32, seed: ptr<function, u32>) -> vec3<f32> {
    let cos_t = mix(1.0, cos(spread), rand01(seed));
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let phi = rand01(seed) * 6.2831853;
    // Orthonormal basis around dir.
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(dir.y) > 0.99);
    let t = normalize(cross(up, dir));
    let b = cross(dir, t);
    return normalize(t * (cos(phi) * sin_t) + b * (sin(phi) * sin_t) + dir * cos_t);
}

@compute @workgroup_size(64)
fn cs_emit(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= em.emit_count) {
        return;
    }
    // Pop a free slot. If the pool is full, undo and skip (emission is best effort).
    // Under contention the counter can briefly wrap below zero, hence the capacity check.
    let before = atomicSub(&counters.dead, 1u);
    if (before == 0u || before > em.capacity) {
        atomicAdd(&counters.dead, 1u);
        return;
    }
    let slot = dead_list[before - 1u];

    var seed = pcg(gid.x ^ (em.frame_seed * 9781u));
    let r = em.origin.w * pow(rand01(&seed), 1.0 / 3.0);
    let offset = random_in_cone(vec3<f32>(0.0, 1.0, 0.0), 3.14159265, &seed) * r;
    var p: Particle;
    p.position = em.origin.xyz + offset;
    p.velocity = random_in_cone(em.direction.xyz, em.direction.w, &seed)
        * mix(em.speed_life.x, em.speed_life.y, rand01(&seed));
    p.age = 0.0;
    p.lifetime = mix(em.speed_life.z, em.speed_life.w, rand01(&seed));
    p.color = em.color_start;
    p.size = em.size_start_end.x;
    p.rotation = rand01(&seed) * 6.2831853;
    p.spin = (rand01(&seed) - 0.5) * 2.0;
    p.seed = rand01(&seed);
    particles[slot] = p;

    let idx = atomicAdd(&counters.alive_cur, 1u);
    alive_cur[idx] = slot;
}

// One thread prepares the indirect dispatch for cs_update from the alive count after emission.
@compute @workgroup_size(1)
fn cs_prepare() {
    let n = atomicLoad(&counters.alive_cur);
    update_args.x = (n + 63u) / 64u;
    update_args.y = 1u;
    update_args.z = 1u;
    atomicStore(&counters.alive_next, 0u);
}

@compute @workgroup_size(64)
fn cs_update(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = atomicLoad(&counters.alive_cur);
    if (gid.x >= n) {
        return;
    }
    let slot = alive_cur[gid.x];
    var p = particles[slot];
    p.age = p.age + em.dt;
    if (p.age >= p.lifetime) {
        let d = atomicAdd(&counters.dead, 1u);
        dead_list[d] = slot;
        return;
    }
    // Semi-implicit Euler with exponential drag.
    p.velocity = (p.velocity + em.gravity_drag.xyz * em.dt) * exp(-em.gravity_drag.w * em.dt);
    p.position = p.position + p.velocity * em.dt;
    p.rotation = p.rotation + p.spin * em.dt;
    let t = p.age / p.lifetime;
    p.color = mix(em.color_start, em.color_end, t);
    p.size = mix(em.size_start_end.x, em.size_start_end.y, t);
    particles[slot] = p;

    let idx = atomicAdd(&counters.alive_next, 1u);
    alive_next[idx] = slot;
}

// The host swaps the alive_cur / alive_next bindings each frame and moves alive_next into alive_cur.
@compute @workgroup_size(1)
fn cs_finish() {
    let n = atomicLoad(&counters.alive_next);
    draw_args.vertex_count = 6u; // one quad per particle, two triangles, no vertex buffer
    draw_args.instance_count = n;
    draw_args.first_vertex = 0u;
    draw_args.first_instance = 0u;
    atomicStore(&counters.alive_cur, n);
}
