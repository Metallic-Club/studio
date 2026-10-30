//! Preview canvases drawn with the egui painter (no GPU work of their own): a 2D bone rig and a
//! blocky 3D figure made of boxes attached to joints, with flat shading and a contact shadow.

use crate::theme::Theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use metallic_anim::glam::{Mat3, Vec2, Vec3};
use metallic_anim::{JointId, Pose, Skeleton, Transform};

fn canvas(p: &egui::Painter, theme: &Theme, rect: Rect, title: &str) {
    p.rect_filled(rect, f32::from(crate::theme::R_SM), theme.deep);
    p.rect_stroke(
        rect,
        f32::from(crate::theme::R_SM),
        Stroke::new(1.0, theme.line),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(12.0),
        theme.text_dim,
    );
}

/// Draws a 2D bone rig (bones along local +X, Y up) fitted into `rect`. Bones whose name
/// contains "Head" get a round head. `highlight` outlines one bone in the accent colour.
pub fn rig_2d(
    ui: &mut egui::Ui,
    rect: Rect,
    theme: &Theme,
    skeleton: &Skeleton,
    pose: &Pose,
    highlight: Option<JointId>,
) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::hover());
    let p = ui.painter_at(rect);
    canvas(&p, theme, rect, "2D bones");
    // Fit from the rest pose so the view does not jump while animating.
    let mut rest_model = Vec::new();
    Pose::rest(skeleton).model_space(skeleton, &mut rest_model);
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for (j, m) in skeleton.joints().iter().zip(&rest_model) {
        for q in [m.translation, m.transform_point(Vec3::X * j.length)] {
            lo = lo.min(q.truncate());
            hi = hi.max(q.truncate());
        }
    }
    if !lo.is_finite() {
        return resp;
    }
    let size = (hi - lo).max(Vec2::splat(0.5));
    let scale = ((rect.width() - 60.0) / size.x).min((rect.height() - 70.0) / size.y) * 0.8;
    let mid = (lo + hi) / 2.0;
    let center = rect.center() + vec2(0.0, 6.0);
    let to_screen = |v: Vec3| {
        pos2(
            center.x + (v.x - mid.x) * scale,
            center.y - (v.y - mid.y) * scale,
        )
    };

    // Ground line and dotted grid.
    let ground = to_screen(Vec3::new(0.0, lo.y, 0.0)).y + 4.0;
    let grid = theme.grid.gamma_multiply(1.6);
    let mut gx = rect.left() + 20.0;
    while gx < rect.right() - 10.0 {
        let mut gy = rect.top() + 30.0;
        while gy < ground - 4.0 {
            p.circle_filled(pos2(gx, gy), 0.9, grid);
            gy += 24.0;
        }
        gx += 24.0;
    }
    p.hline(
        (rect.left() + 24.0)..=(rect.right() - 24.0),
        ground,
        Stroke::new(1.0, theme.line),
    );

    let mut model = Vec::new();
    pose.model_space(skeleton, &mut model);
    let bone_fill = theme.silver;
    // Links from each child's root to the nearest point on its parent bone (shoulders, hips), so
    // offset joints read as attached.
    for (i, j) in skeleton.joints().iter().enumerate() {
        let Some(parent) = j.parent else { continue };
        let (pm, pj) = (&model[parent], &skeleton.joints()[parent]);
        let a = to_screen(pm.translation);
        let b = to_screen(pm.transform_point(Vec3::X * pj.length));
        let c = to_screen(model[i].translation);
        let ab = b - a;
        let s = if ab.length_sq() > 1e-3 {
            ((c - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let near = a + ab * s;
        if near.distance(c) > 2.0 {
            p.line_segment([near, c], Stroke::new(2.0, theme.text_meta));
        }
    }
    let bone_shade = Color32::from_rgb(0x9c, 0x99, 0xa6);
    // Draw parents first; heads last so they sit on top of the neck.
    let mut order: Vec<usize> = (0..skeleton.len()).collect();
    order.sort_by_key(|&i| skeleton.joints()[i].name.contains("Head"));
    for &i in &order {
        let j = &skeleton.joints()[i];
        let m: &Transform = &model[i];
        let a = to_screen(m.translation);
        let b = to_screen(m.transform_point(Vec3::X * j.length));
        let selected = highlight == Some(i);
        let outline = Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected { theme.accent } else { theme.deep },
        );
        if j.name.contains("Head") {
            let c = pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            let r = (a.distance(b) / 2.0).max(6.0);
            p.circle(c, r, bone_fill, outline);
            // A small facing mark.
            let dir = (b - a).normalized();
            p.circle_filled(
                c + dir.rot90() * (r * 0.35) + dir * (r * 0.2),
                r * 0.12,
                theme.deep,
            );
            continue;
        }
        let d = b - a;
        let len = d.length();
        if len < 1.0 {
            p.circle(a, 5.0, bone_fill, outline);
            continue;
        }
        let n = d.rot90() / len;
        let w = (len * 0.16).clamp(4.0, 13.0);
        let kink = a + d * 0.18;
        let poly = vec![a, kink + n * w, b, kink - n * w];
        p.add(Shape::convex_polygon(poly.clone(), bone_fill, outline));
        // A darker half gives the bone a bevel.
        p.add(Shape::convex_polygon(
            vec![a, kink - n * w, b],
            bone_shade,
            Stroke::NONE,
        ));
        p.circle(
            a,
            (w * 0.55).max(3.0),
            theme.raised,
            Stroke::new(1.2, if selected { theme.accent } else { bone_fill }),
        );
    }
    resp
}

/// A box attached to a joint, for [`figure_3d`].
#[derive(Debug, Clone, PartialEq)]
pub struct BoxPart {
    /// Joint the box follows.
    pub joint: JointId,
    /// Box centre in the joint's local space.
    pub center: Vec3,
    /// Box size.
    pub size: Vec3,
    /// Base colour (shaded by the light).
    pub color: Color32,
}

/// Orbit camera for [`figure_3d`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    /// Rotation about the vertical axis, radians.
    pub yaw: f32,
    /// Elevation, radians.
    pub pitch: f32,
    /// Distance from the target.
    pub distance: f32,
    /// Point looked at.
    pub target: Vec3,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.55,
            pitch: 0.28,
            distance: 13.0,
            target: Vec3::new(0.0, 2.6, 0.0),
        }
    }
}

/// Draws a blocky figure in perspective with flat shading, a ground grid and a soft contact
/// shadow. Drag to orbit. Returns the canvas response.
#[allow(clippy::too_many_arguments)]
pub fn figure_3d(
    ui: &mut egui::Ui,
    rect: Rect,
    theme: &Theme,
    skeleton: &Skeleton,
    pose: &Pose,
    parts: &[BoxPart],
    camera: &mut OrbitCamera,
    highlight: Option<JointId>,
) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::drag());
    if resp.dragged() {
        let d = resp.drag_delta();
        camera.yaw -= d.x * 0.01;
        camera.pitch = (camera.pitch + d.y * 0.01).clamp(-0.2, 1.3);
    }
    let p = ui.painter_at(rect);
    canvas(&p, theme, rect, "3D figure  ·  drag to orbit");

    let eye = camera.target
        + Vec3::new(
            camera.yaw.sin() * camera.pitch.cos(),
            camera.pitch.sin(),
            camera.yaw.cos() * camera.pitch.cos(),
        ) * camera.distance;
    let fwd = (camera.target - eye).normalize_or_zero();
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(fwd);
    let view = Mat3::from_cols(right, up, -fwd).transpose();
    let focal = rect.height() * 1.25;
    let center = rect.center() + vec2(0.0, 10.0);
    let project = |w: Vec3| -> Option<(Pos2, f32)> {
        let v = view * (w - eye);
        let z = -v.z;
        if z < 0.1 {
            return None;
        }
        Some((
            pos2(center.x + v.x / z * focal, center.y - v.y / z * focal),
            z,
        ))
    };

    // Ground grid.
    for i in -6..=6 {
        let f = i as f32;
        let alpha = 1.0 - (f.abs() / 7.0);
        let col = theme.line.gamma_multiply(0.35 + 0.45 * alpha);
        if let (Some(a), Some(b)) = (
            project(Vec3::new(f, 0.0, -6.0)),
            project(Vec3::new(f, 0.0, 6.0)),
        ) {
            p.line_segment([a.0, b.0], Stroke::new(1.0, col));
        }
        if let (Some(a), Some(b)) = (
            project(Vec3::new(-6.0, 0.0, f)),
            project(Vec3::new(6.0, 0.0, f)),
        ) {
            p.line_segment([a.0, b.0], Stroke::new(1.0, col));
        }
    }

    let mut model = Vec::new();
    pose.model_space(skeleton, &mut model);

    // Contact shadow under the figure: stacked translucent ellipses on the ground.
    let root = model.first().map_or(Vec3::ZERO, |m| m.translation);
    let height = root.y.max(0.0);
    for (k, a) in [(1.0f32, 18u8), (0.75, 22), (0.5, 26)] {
        let r = (1.9 - height * 0.15).max(0.6) * k;
        let pts: Vec<Pos2> = (0..32)
            .filter_map(|i| {
                let ang = i as f32 / 32.0 * std::f32::consts::TAU;
                project(Vec3::new(
                    root.x + ang.cos() * r * 1.2,
                    0.0,
                    root.z + ang.sin() * r,
                ))
                .map(|q| q.0)
            })
            .collect();
        if pts.len() == 32 {
            p.add(Shape::convex_polygon(
                pts,
                Color32::from_black_alpha(a),
                Stroke::NONE,
            ));
        }
    }

    // Faces of every box, sorted back to front.
    let light = Vec3::new(-0.45, 0.8, 0.55).normalize();
    let mut faces: Vec<(f32, Vec<Pos2>, Color32, bool)> = Vec::new();
    const FACES: [([usize; 4], Vec3); 6] = [
        ([0, 1, 3, 2], Vec3::NEG_X),
        ([4, 6, 7, 5], Vec3::X),
        ([0, 4, 5, 1], Vec3::NEG_Y),
        ([2, 3, 7, 6], Vec3::Y),
        ([0, 2, 6, 4], Vec3::NEG_Z),
        ([1, 5, 7, 3], Vec3::Z),
    ];
    for part in parts {
        let Some(m) = model.get(part.joint) else {
            continue;
        };
        let h = part.size / 2.0;
        let corners: Vec<Vec3> = (0..8)
            .map(|i| {
                let s = Vec3::new(
                    if i & 4 != 0 { 1.0 } else { -1.0 },
                    if i & 2 != 0 { 1.0 } else { -1.0 },
                    if i & 1 != 0 { 1.0 } else { -1.0 },
                );
                m.transform_point(part.center + s * h)
            })
            .collect();
        for (idx, n) in FACES {
            let normal = (m.rotation * n).normalize_or_zero();
            let face_center = idx.iter().map(|&i| corners[i]).sum::<Vec3>() / 4.0;
            if normal.dot(eye - face_center) <= 0.0 {
                continue;
            }
            let pts: Option<Vec<(Pos2, f32)>> = idx.iter().map(|&i| project(corners[i])).collect();
            let Some(pts) = pts else { continue };
            let depth = pts.iter().map(|q| q.1).sum::<f32>() / 4.0;
            let lambert = normal.dot(light).max(0.0);
            let shade = 0.42 + 0.58 * lambert + 0.08 * normal.y.max(0.0);
            let c = part.color;
            let col = Color32::from_rgb(
                (f32::from(c.r()) * shade).min(255.0) as u8,
                (f32::from(c.g()) * shade).min(255.0) as u8,
                (f32::from(c.b()) * shade).min(255.0) as u8,
            );
            faces.push((
                depth,
                pts.into_iter().map(|q| q.0).collect(),
                col,
                highlight == Some(part.joint),
            ));
        }
    }
    faces.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, pts, col, hi) in faces {
        let stroke = if hi {
            Stroke::new(1.5, theme.accent)
        } else {
            Stroke::new(0.8, Color32::from_black_alpha(90))
        };
        p.add(Shape::convex_polygon(pts, col, stroke));
    }
    resp
}
