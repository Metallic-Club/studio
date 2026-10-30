//! Turning a simulation into textured quads: a renderer-neutral draw list.
//!
//! [`build_draw_list`] expands every particle, ribbon, trail segment and beam segment into a [`Quad`]
//! (four world-space corners, UVs, per-corner premultiplied colour) sorted back to front. The egui preview
//! projects and paints these; the wgpu renderer will do the same expansion in a vertex shader from the
//! particle streams, so this module doubles as the reference for that shader.
//!
//! Blending follows Roblox `LightEmission`: colours are premultiplied, and alpha is scaled by
//! `1 - light_emission`. Alpha 0 with a non-zero colour adds light; alpha 1 covers. One blend state
//! (`src + dst * (1 - src_alpha)`) therefore draws both kinds in a single sorted pass.

use crate::color::Rgb;
use crate::curve::Curve;
use crate::effect::{
    Beam, Emitter, Facing, Flipbook, FlipbookMode, Render, RenderKind, SimSpace, SortMode,
    Texture, TextureMode, Trail,
};
use crate::math::{Transform, Vec3};
use crate::rng::{Rng, pcg};
use crate::sim::{EmitterState, Simulation, TrailState};

/// A view to draw from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Eye position.
    pub position: Vec3,
    /// Point looked at.
    pub target: Vec3,
    /// Vertical field of view in degrees (perspective).
    pub fov_y_deg: f32,
    /// When set, an orthographic view this many studs tall (2D games).
    pub ortho_height: Option<f32>,
}

impl Camera {
    /// A perspective camera.
    #[must_use]
    pub fn perspective(position: Vec3, target: Vec3, fov_y_deg: f32) -> Self {
        Self {
            position,
            target,
            fov_y_deg,
            ortho_height: None,
        }
    }

    /// An orthographic camera `height` studs tall.
    #[must_use]
    pub fn orthographic(position: Vec3, target: Vec3, height: f32) -> Self {
        Self {
            position,
            target,
            fov_y_deg: 50.0,
            ortho_height: Some(height),
        }
    }

    /// Right, up and forward unit vectors.
    #[must_use]
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.target - self.position).normalize_or(-Vec3::Z);
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward);
        (right, up, forward)
    }

    /// Unit vector from `p` towards the eye (or against the view direction for orthographic views).
    #[must_use]
    pub fn to_eye(&self, p: Vec3) -> Vec3 {
        if self.ortho_height.is_some() {
            -self.basis().2
        } else {
            (self.position - p).normalize_or(-self.basis().2)
        }
    }

    /// Projects a world point to pixel coordinates in a `width` x `height` viewport (origin top-left)
    /// plus view depth. `None` when the point is behind the near plane.
    #[must_use]
    pub fn project(&self, p: Vec3, width: f32, height: f32) -> Option<[f32; 3]> {
        let (right, up, forward) = self.basis();
        let v = p - self.position;
        let (x, y, z) = (v.dot(right), v.dot(up), v.dot(forward));
        let aspect = width / height.max(1.0);
        let (nx, ny) = match self.ortho_height {
            Some(h) => (x / (h * 0.5 * aspect), y / (h * 0.5)),
            None => {
                if z < 0.05 {
                    return None;
                }
                let t = (self.fov_y_deg.to_radians() * 0.5).tan();
                (x / (z * t * aspect), y / (z * t))
            }
        };
        Some([(nx + 1.0) * 0.5 * width, (1.0 - ny) * 0.5 * height, z])
    }
}

/// One textured quad, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// World-space corners, in order around the quad.
    pub corners: [Vec3; 4],
    /// Texture coordinates per corner.
    pub uvs: [[f32; 2]; 4],
    /// Premultiplied sRGB colour per corner; alpha 0 means purely additive.
    pub colors: [[f32; 4]; 4],
    /// Index into [`DrawList::textures`].
    pub texture: u16,
    /// View depth used for sorting (larger is farther).
    pub depth: f32,
    /// Snap projected corners to a grid of this many pixels (0 = off).
    pub pixel_snap: f32,
}

/// Quads plus the textures they use, sorted back to front.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    /// Textures referenced by [`Quad::texture`].
    pub textures: Vec<Texture>,
    /// Quads, far to near.
    pub quads: Vec<Quad>,
}

impl DrawList {
    fn texture_index(&mut self, t: &Texture) -> u16 {
        if let Some(i) = self.textures.iter().position(|x| x == t) {
            return i as u16;
        }
        self.textures.push(t.clone());
        (self.textures.len() - 1) as u16
    }
}

/// Premultiplies a straight colour with Roblox-style light emission and brightness.
#[must_use]
pub fn premultiply(rgb: Rgb, alpha: f32, light_emission: f32, brightness: f32) -> [f32; 4] {
    let a = alpha.clamp(0.0, 1.0);
    let b = brightness.max(0.0);
    [
        rgb.r * a * b,
        rgb.g * a * b,
        rgb.b * a * b,
        a * (1.0 - light_emission.clamp(0.0, 1.0)),
    ]
}

/// Quantises opacity for toon and pixel looks (`steps` 0 = unchanged, 1 = cut-out at 0.5).
#[must_use]
pub fn step_alpha(a: f32, steps: u32) -> f32 {
    match steps {
        0 => a,
        1 => {
            if a >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        n => (a * n as f32).round() / n as f32,
    }
}

/// Flipbook frames for a particle: `(frame a, frame b, blend 0..1 from a to b)`.
#[must_use]
pub fn flipbook_frames(fb: &Flipbook, age: f32, life: f32, random: f32, id: u32) -> (u32, u32, f32) {
    let n = fb.frames();
    if n <= 1 {
        return (0, 0, 0.0);
    }
    let fps = fb.fps.at(random).clamp(0.0, 60.0);
    let start = if fb.start_random {
        (random * 7919.0).fract() * n as f32
    } else {
        0.0
    };
    let nf = n as f32;
    let (a, b, f) = match fb.mode {
        FlipbookMode::OneShot => {
            let t = (age / life.max(1e-4)).clamp(0.0, 1.0);
            if fb.blend {
                let x = t * (nf - 1.0);
                let i = x.floor();
                (i, (i + 1.0).min(nf - 1.0), x - i)
            } else {
                ((t * nf).floor().min(nf - 1.0), 0.0, 0.0)
            }
        }
        FlipbookMode::Loop => {
            let x = (start + age * fps).rem_euclid(nf);
            let i = x.floor();
            (i, (i + 1.0) % nf, x - i)
        }
        FlipbookMode::PingPong => {
            let period = 2 * (n - 1);
            let x = start + age * fps;
            let k = x.floor() as u32;
            let frame = |k: u32| {
                let k = k % period;
                (if k < n { k } else { period - k }) as f32
            };
            (frame(k), frame(k + 1), x.fract())
        }
        FlipbookMode::Random => {
            let x = age * fps;
            let k = x.floor() as u32;
            let pick = |k: u32| (pcg(id ^ pcg(k)) % n) as f32;
            (pick(k), pick(k + 1), x.fract())
        }
    };
    let blend = if fb.blend { f } else { 0.0 };
    (a as u32, b as u32, blend)
}

fn frame_uv(frame: u32, columns: u32, rows: u32) -> [f32; 4] {
    let c = columns.max(1);
    let r = rows.max(1);
    let col = frame % c;
    let row = (frame / c) % r;
    [
        col as f32 / c as f32,
        row as f32 / r as f32,
        (col + 1) as f32 / c as f32,
        (row + 1) as f32 / r as f32,
    ]
}

fn rect_uvs(r: [f32; 4]) -> [[f32; 2]; 4] {
    // Corner order: bottom-left, bottom-right, top-right, top-left (V grows downwards in images).
    [[r[0], r[3]], [r[2], r[3]], [r[2], r[1]], [r[0], r[1]]]
}

/// The flipbook an emitter effectively uses: its own, or a default for sheet textures.
#[must_use]
pub fn effective_flipbook(render: &Render) -> Option<Flipbook> {
    if let Some(fb) = &render.flipbook {
        return Some(fb.clone());
    }
    render.texture.sheet().map(|(columns, rows)| Flipbook {
        columns,
        rows,
        mode: FlipbookMode::OneShot,
        fps: crate::curve::Range::constant(12.0),
        start_random: false,
        blend: true,
    })
}

/// Builds the sorted draw list for a whole simulation.
#[must_use]
pub fn build_draw_list(sim: &Simulation, camera: &Camera) -> DrawList {
    let mut out = DrawList::default();
    let effect = sim.effect();
    let xf = sim.transform();
    let (_, _, forward) = camera.basis();
    for (em, state) in effect.emitters.iter().zip(sim.emitters()) {
        if !em.enabled || state.pool.is_empty() {
            continue;
        }
        let first = out.quads.len();
        match &em.render.kind {
            RenderKind::Ribbon {
                texture_mode,
                texture_length,
            } => ribbon_quads(
                em,
                state,
                camera,
                &xf,
                *texture_mode,
                *texture_length,
                &mut out,
            ),
            _ => sprite_quads(em, state, camera, &xf, &mut out),
        }
        apply_sort_mode(em.render.sort, &mut out.quads[first..], camera, forward);
    }
    for (trail, state) in effect.trails.iter().zip(sim.trails()) {
        trail_quads(trail, state, camera, &mut out);
    }
    for (i, beam) in effect.beams.iter().enumerate() {
        if beam.enabled {
            beam_quads(beam, i as u32, sim.seed(), sim.time(), &xf, camera, &mut out);
        }
    }
    out.quads.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

fn apply_sort_mode(mode: SortMode, quads: &mut [Quad], camera: &Camera, forward: Vec3) {
    if mode == SortMode::ByDistance || quads.is_empty() {
        return;
    }
    // Keep the emitter as one depth layer and order inside it by age (quads were emitted in pool order;
    // the caller's later sort is stable, so tiny offsets keep this order).
    let mean = quads
        .iter()
        .map(|q| ((q.corners[0] + q.corners[2]) * 0.5 - camera.position).dot(forward))
        .sum::<f32>()
        / quads.len() as f32;
    let n = quads.len() as f32;
    for (rank, q) in quads.iter_mut().enumerate() {
        let r = rank as f32 / n;
        q.depth = mean + 1e-3 * if mode == SortMode::NewestOnTop { r } else { -r };
    }
}

#[allow(clippy::too_many_lines)]
fn sprite_quads(
    em: &Emitter,
    state: &EmitterState,
    camera: &Camera,
    xf: &Transform,
    out: &mut DrawList,
) {
    let r = &em.render;
    let tex = out.texture_index(&r.texture);
    let fb = effective_flipbook(r);
    let (cam_right, cam_up, forward) = camera.basis();
    let local = em.space == SimSpace::Local;
    let pool = &state.pool;
    // Sort by age within the emitter so OldestOnTop/NewestOnTop have a defined order.
    let mut order: Vec<usize> = (0..pool.len()).collect();
    if r.sort != SortMode::ByDistance {
        order.sort_by(|&a, &b| {
            pool.position_age[b][3]
                .total_cmp(&pool.position_age[a][3])
                .then(pool.ids[a].cmp(&pool.ids[b]))
        });
    }
    for i in order {
        let pa = pool.position_age[i];
        let vl = pool.velocity_life[i];
        let vis = pool.visual[i];
        let tint = pool.tint[i];
        let motion = pool.motion[i];
        let mut p = Vec3::new(pa[0], pa[1], pa[2]);
        let mut v = Vec3::new(vl[0], vl[1], vl[2]);
        if local {
            p = xf.point(p);
            v = xf.rotate(v);
        }
        let alpha = step_alpha(tint[3], r.alpha_steps);
        if alpha <= 0.0 {
            continue;
        }
        let shrink = 1.0 - vis[2];
        let (w, h) = (vis[0] * shrink, vis[1] * shrink);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let to_eye = camera.to_eye(p);
        let center = p + to_eye * r.z_offset;
        let speed = v.length();
        let vdir = v.normalize_or(Vec3::Y);
        let rot = motion[0];
        let spin = |a: Vec3, b: Vec3| {
            let (s, c) = rot.sin_cos();
            (a * c + b * s, b * c - a * s)
        };
        let (axis_x, axis_y, half_w, half_h, center) = match (&r.kind, r.facing) {
            (RenderKind::Stretched {
                speed_scale,
                min_length,
            }, _) => {
                let up = (vdir - to_eye * vdir.dot(to_eye)).normalize_or(cam_up);
                let right = up.cross(to_eye).normalize_or(cam_right);
                let len = (h + speed * speed_scale).max(h * min_length);
                (right, up, w * 0.5, len * 0.5, center - up * ((len - h) * 0.5))
            }
            (_, Facing::Camera) => {
                let (a, b) = spin(cam_right, cam_up);
                (a, b, w * 0.5, h * 0.5, center)
            }
            (_, Facing::CameraWorldUp) => {
                let right = Vec3::Y.cross(to_eye).normalize_or(cam_right);
                let (a, b) = spin(right, Vec3::Y);
                (a, b, w * 0.5, h * 0.5, center)
            }
            (_, Facing::VelocityParallel) => {
                let up = (vdir - to_eye * vdir.dot(to_eye)).normalize_or(cam_up);
                let right = up.cross(to_eye).normalize_or(cam_right);
                (right, up, w * 0.5, h * 0.5, center)
            }
            (_, Facing::VelocityPerpendicular) => {
                let (t, b) = vdir.basis();
                let (a, b) = spin(t, b);
                (a, b, w * 0.5, h * 0.5, center)
            }
            (_, Facing::Ground) => {
                let (a, b) = spin(Vec3::X, -Vec3::Z);
                (a, b, w * 0.5, h * 0.5, center)
            }
        };
        let corners = [
            center - axis_x * half_w - axis_y * half_h,
            center + axis_x * half_w - axis_y * half_h,
            center + axis_x * half_w + axis_y * half_h,
            center - axis_x * half_w + axis_y * half_h,
        ];
        let depth = (center - camera.position).dot(forward);
        let rgb = Rgb::new(tint[0], tint[1], tint[2]);
        match &fb {
            Some(fb) => {
                let (fa, fbn, blend) = flipbook_frames(fb, pa[3], vl[3], motion[3], pool.ids[i]);
                let layers = if blend > 0.0 && fa != fbn {
                    vec![(fa, 1.0 - blend), (fbn, blend)]
                } else {
                    vec![(fa, 1.0)]
                };
                for (frame, weight) in layers {
                    let c = premultiply(rgb, alpha * weight, r.light_emission, r.brightness);
                    out.quads.push(Quad {
                        corners,
                        uvs: rect_uvs(frame_uv(frame, fb.columns, fb.rows)),
                        colors: [c; 4],
                        texture: tex,
                        depth,
                        pixel_snap: r.pixel_snap,
                    });
                }
            }
            None => {
                let c = premultiply(rgb, alpha, r.light_emission, r.brightness);
                out.quads.push(Quad {
                    corners,
                    uvs: rect_uvs([0.0, 0.0, 1.0, 1.0]),
                    colors: [c; 4],
                    texture: tex,
                    depth,
                    pixel_snap: r.pixel_snap,
                });
            }
        }
    }
}

/// Adds the quads of a strip through `points` (`(position, half width, colour premultiplied, u)`).
fn strip(
    points: &[(Vec3, f32, [f32; 4], f32)],
    side_of: &dyn Fn(usize, Vec3) -> Vec3,
    tex: u16,
    snap: f32,
    camera: &Camera,
    out: &mut DrawList,
) {
    let (_, _, forward) = camera.basis();
    for i in 0..points.len().saturating_sub(1) {
        let (p0, w0, c0, u0) = points[i];
        let (p1, w1, c1, u1) = points[i + 1];
        let dir = (p1 - p0).normalize_or(Vec3::X);
        let s0 = side_of(i, dir) * w0;
        let s1 = side_of(i + 1, dir) * w1;
        let corners = [p0 - s0, p1 - s1, p1 + s1, p0 + s0];
        let depth = ((p0 + p1) * 0.5 - camera.position).dot(forward);
        out.quads.push(Quad {
            corners,
            uvs: [[u0, 1.0], [u1, 1.0], [u1, 0.0], [u0, 0.0]],
            colors: [c0, c1, c1, c0],
            texture: tex,
            depth,
            pixel_snap: snap,
        });
    }
}

fn ribbon_quads(
    em: &Emitter,
    state: &EmitterState,
    camera: &Camera,
    xf: &Transform,
    mode: TextureMode,
    length: f32,
    out: &mut DrawList,
) {
    let r = &em.render;
    let pool = &state.pool;
    let mut order: Vec<usize> = (0..pool.len()).collect();
    order.sort_by_key(|&i| pool.ids[i]);
    let local = em.space == SimSpace::Local;
    let pos = |i: usize| {
        let p = pool.position(i);
        if local { xf.point(p) } else { p }
    };
    let mut total = 0.0;
    for w in order.windows(2) {
        total += (pos(w[1]) - pos(w[0])).length();
    }
    let mut points = Vec::with_capacity(order.len());
    let mut run = 0.0;
    for (k, &i) in order.iter().enumerate() {
        if k > 0 {
            run += (pos(i) - pos(order[k - 1])).length();
        }
        let tint = pool.tint[i];
        let alpha = step_alpha(tint[3], r.alpha_steps);
        let c = premultiply(
            Rgb::new(tint[0], tint[1], tint[2]),
            alpha,
            r.light_emission,
            r.brightness,
        );
        let u = match mode {
            TextureMode::Stretch => run / total.max(1e-4) * length,
            TextureMode::Wrap | TextureMode::Static => run / length.max(1e-3),
        };
        points.push((pos(i), pool.visual[i][0] * 0.5, c, u));
    }
    let tex = out.texture_index(&r.texture);
    let side = |k: usize, dir: Vec3| {
        let p = points.get(k).map_or(Vec3::ZERO, |q| q.0);
        dir.cross(camera.to_eye(p)).normalize_or(Vec3::Y)
    };
    strip(&points, &side, tex, r.pixel_snap, camera, out);
}

fn trail_quads(trail: &Trail, state: &TrailState, camera: &Camera, out: &mut DrawList) {
    if state.points.len() < 2 {
        return;
    }
    let life = trail.lifetime.max(0.01);
    let centers: Vec<Vec3> = state.points.iter().map(|p| (p.a + p.b) * 0.5).collect();
    let mut total = 0.0;
    for w in centers.windows(2) {
        total += (w[1] - w[0]).length();
    }
    if total <= 1e-5 {
        return;
    }
    let mut run = 0.0;
    let mut points = Vec::with_capacity(centers.len());
    for (k, p) in state.points.iter().enumerate() {
        if k > 0 {
            run += (centers[k] - centers[k - 1]).length();
        }
        let along = run / total;
        let width = (p.a - p.b).length() * trail.width_scale.sample(along).max(0.0);
        let alpha = 1.0 - trail.transparency.sample(p.age / life).clamp(0.0, 1.0);
        let c = premultiply(
            trail.color.sample(along),
            alpha,
            trail.light_emission,
            trail.brightness,
        );
        let u = match trail.texture_mode {
            TextureMode::Stretch => along * trail.texture_length,
            TextureMode::Wrap => run / trail.texture_length.max(1e-3),
            TextureMode::Static => (total - run) / trail.texture_length.max(1e-3),
        };
        points.push((centers[k], width * 0.5, c, u));
    }
    let tex = out.texture_index(&trail.texture);
    let pts = &state.points;
    let face = trail.face_camera;
    let side = |k: usize, dir: Vec3| {
        if face {
            dir.cross(camera.to_eye(centers[k])).normalize_or(Vec3::Y)
        } else {
            (pts[k].a - pts[k].b).normalize_or(Vec3::Y)
        }
    };
    strip(&points, &side, tex, 0.0, camera, out);
}

/// Point on a beam's Bezier curve in local space (Roblox control point rules).
#[must_use]
pub fn beam_point(beam: &Beam, t: f32) -> Vec3 {
    let p0 = beam.attachment0;
    let p3 = beam.attachment1;
    let p1 = p0 + beam.direction0.normalize_or(Vec3::X) * beam.curve_size0;
    let p2 = p3 - beam.direction1.normalize_or(Vec3::X) * beam.curve_size1;
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

fn beam_quads(
    beam: &Beam,
    index: u32,
    seed: u32,
    time: f32,
    xf: &Transform,
    camera: &Camera,
    out: &mut DrawList,
) {
    let n = beam.segments.clamp(1, 1000);
    let jitter_frame = if beam.jitter_fps > 0.0 {
        (time * beam.jitter_fps).floor() as u32
    } else {
        0
    };
    let offset = -time * beam.texture_speed;
    let mut world: Vec<Vec3> = Vec::with_capacity(n as usize + 1);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let mut p = beam_point(beam, t);
        if beam.jitter > 0.0 && k > 0 && k < n {
            let mut rng = Rng::for_event(seed ^ index, jitter_frame, k);
            let taper = (t * std::f32::consts::PI).sin();
            p += rng.unit_vector() * (beam.jitter * taper * rng.next_f32().sqrt());
        }
        world.push(xf.point(p));
    }
    let mut total = 0.0;
    for w in world.windows(2) {
        total += (w[1] - w[0]).length();
    }
    let mut run = 0.0;
    let mut points = Vec::with_capacity(world.len());
    for (k, &p) in world.iter().enumerate() {
        if k > 0 {
            run += (p - world[k - 1]).length();
        }
        let t = k as f32 / n as f32;
        let width = beam.width0 + (beam.width1 - beam.width0) * t;
        let alpha = 1.0 - beam.transparency.sample(t).clamp(0.0, 1.0);
        let c = premultiply(beam.color.sample(t), alpha, beam.light_emission, beam.brightness);
        let u = match beam.texture_mode {
            TextureMode::Stretch => t * beam.texture_length,
            TextureMode::Wrap | TextureMode::Static => run / beam.texture_length.max(1e-3),
        } + offset;
        let toward = camera.to_eye(p) * beam.z_offset;
        points.push((p + toward, width.abs() * 0.5, c, u));
    }
    let _ = total;
    let tex = out.texture_index(&beam.texture);
    let face = beam.face_camera;
    let up = xf.up;
    let side = |k: usize, dir: Vec3| {
        if face {
            dir.cross(camera.to_eye(world[k])).normalize_or(Vec3::Y)
        } else {
            dir.cross(up).normalize_or(Vec3::Z)
        }
    };
    strip(&points, &side, tex, 0.0, camera, out);
}

/// Evaluates a [`Curve`] at `n + 1` evenly spaced points (for widgets and tests).
#[must_use]
pub fn tabulate(curve: &Curve, n: usize) -> Vec<f32> {
    (0..=n).map(|i| curve.sample(i as f32 / n.max(1) as f32)).collect()
}
