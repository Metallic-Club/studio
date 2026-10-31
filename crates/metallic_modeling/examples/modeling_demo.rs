//! Modeller demo: an egui viewport over `metallic_modeling` with vertex / edge / face modes and the core operations
//! on keys. No renderer dependency: faces are flat-shaded egui triangles sorted back to front.
//!
//! Run:        `cargo run -p metallic_modeling --example modeling_demo`
//! Screenshot: `cargo run -p metallic_modeling --example modeling_demo -- --screenshot out.png [faces|edges|verts]`
//!             (renders headless through egui_kittest + wgpu; no window opens)
//!
//! Keys (Blender-like, with on-screen hints): 1/2/3 vertex/edge/face mode, click select, Shift+click add,
//! Alt+click edge loop, A select all, Alt+A none, G move, R rotate, S scale, E extrude, I inset, Ctrl+B bevel
//! (wheel: segments), Ctrl+R loop cut (wheel: cuts), F fill, M merge, X delete, Ctrl+Z / Ctrl+Y undo / redo,
//! middle-drag or Alt+left-drag orbit, Shift+middle pan, wheel zoom, Home frame all, Alt+Z x-ray.
//!
//! Theme: graphite tokens from the Studio UI research (PR #127, section 5.13). Fonts: `default_fonts` is off
//! (licence), so the demo loads the system UI font at run time (Segoe UI on Windows); IBM Plex is pending the owner's
//! OFL decision.

use std::collections::BTreeSet;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Key,
    Margin, Pos2, Rect, Sense, Stroke, Vec2,
};
use glam::{Mat4, Quat, Vec3, Vec4Swizzles};
use metallic_modeling::ops::loops::{edge_loop, edge_ring};
use metallic_modeling::{
    DeleteKind, EdgeId, EditMesh, FaceId, History, MeshCommand, NormalMode, Op, Primitive,
    SelectMode, Selection, Symmetry, Transform, Triangulated, VertId, contextual_extrude,
    triangulate,
};

// ---------------------------------------------------------------------------------------------------------------
// Theme tokens (PR #127, 5.13)
// ---------------------------------------------------------------------------------------------------------------

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const BG_DEEP: Color32 = hex(0x111015);
const BG_BASE: Color32 = hex(0x16151b);
const BG_PANEL: Color32 = hex(0x1c1b22);
const BG_RAISED: Color32 = hex(0x24222b);
const BG_HOVER: Color32 = hex(0x2c2a34);
const BG_ACTIVE: Color32 = hex(0x35323e);
const SELECT: Color32 = hex(0x3a3354);
const LINE: Color32 = hex(0x2b2933);
const LINE_STRONG: Color32 = hex(0x3b3845);
const TEXT: Color32 = hex(0xe6e4ea);
const TEXT_2: Color32 = hex(0xaeaab6);
const TEXT_3: Color32 = hex(0x8e8a97);
const ICON: Color32 = hex(0xbdbac4);
const ACCENT: Color32 = hex(0x9d8be0);
const ACCENT_TEXT: Color32 = hex(0xb4a6ec);
const ERROR: Color32 = hex(0xec7a72);
const WARNING: Color32 = hex(0xe0ad55);
const INFO: Color32 = hex(0x7fb0e6);
const AXIS_X: Color32 = hex(0xe5675e);
const AXIS_Y: Color32 = hex(0x79c46a);
const AXIS_Z: Color32 = hex(0x5d95ea);
/// Model surface: titanium grey, lit in the viewport.
const SURFACE: [f32; 3] = [0.56, 0.55, 0.60];

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BG_PANEL;
    v.window_fill = BG_RAISED;
    v.window_stroke = Stroke::new(1.0, LINE_STRONG);
    v.extreme_bg_color = BG_DEEP;
    v.faint_bg_color = BG_RAISED;
    v.code_bg_color = BG_DEEP;
    v.hyperlink_color = ACCENT_TEXT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.weak_text_color = Some(TEXT_3);
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_fill = BG_PANEL;
    v.widgets.noninteractive.weak_bg_fill = BG_PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
    for (w, fill) in [
        (&mut v.widgets.inactive, BG_RAISED),
        (&mut v.widgets.hovered, BG_HOVER),
        (&mut v.widgets.active, BG_ACTIVE),
        (&mut v.widgets.open, BG_ACTIVE),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0, TEXT);
        w.corner_radius = CornerRadius::same(4);
        w.expansion = 0.0;
    }
    style.spacing.item_spacing = Vec2::new(6.0, 5.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    ctx.set_global_style(style);
}

/// `default_fonts` is off (its licence includes OFL/Ubuntu-font); load the platform UI font at run time.
fn load_fonts(ctx: &egui::Context) {
    let candidates: &[(&str, &[&str])] = &[
        (
            "ui",
            &[
                "C:/Windows/Fonts/segoeui.ttf",
                "/System/Library/Fonts/SFNS.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ],
        ),
        (
            "ui-semibold",
            &[
                "C:/Windows/Fonts/seguisb.ttf",
                "C:/Windows/Fonts/segoeuib.ttf",
            ],
        ),
        (
            "mono",
            &[
                "C:/Windows/Fonts/consola.ttf",
                "/System/Library/Fonts/Menlo.ttc",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            ],
        ),
    ];
    let mut fonts = FontDefinitions::empty();
    for (name, paths) in candidates {
        if let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert(
                (*name).to_owned(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
        }
    }
    let has = |n: &str| fonts.font_data.contains_key(n);
    let mut prop = Vec::new();
    if has("ui") {
        prop.push("ui".to_owned());
    }
    let mut mono = Vec::new();
    if has("mono") {
        mono.push("mono".to_owned());
    }
    mono.extend(prop.iter().cloned());
    prop.extend(
        mono.iter()
            .filter(|m| !prop.contains(m))
            .cloned()
            .collect::<Vec<_>>(),
    );
    let mut bold = if has("ui-semibold") {
        vec!["ui-semibold".to_owned()]
    } else {
        Vec::new()
    };
    bold.extend(prop.iter().cloned());
    if prop.is_empty() {
        eprintln!("modeling_demo: no system font found; text will not render");
    }
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), bold);
    ctx.set_fonts(fonts);
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

// ---------------------------------------------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
    target: Vec3,
}

impl Camera {
    fn eye(&self) -> Vec3 {
        let dir = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        self.target + dir * self.distance
    }

    fn view_proj(&self, rect: Rect) -> Mat4 {
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.1);
        let proj =
            glam::camera::rh::proj::directx::perspective(40f32.to_radians(), aspect, 0.05, 500.0);
        proj * glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }
}

/// Projects world points into a screen rect.
struct Projector {
    vp: Mat4,
    inv: Mat4,
    rect: Rect,
    eye: Vec3,
}

impl Projector {
    fn new(cam: &Camera, rect: Rect) -> Self {
        let vp = cam.view_proj(rect);
        Self {
            vp,
            inv: vp.inverse(),
            rect,
            eye: cam.eye(),
        }
    }

    /// Screen position and depth (clip w), or `None` behind the camera.
    fn project(&self, p: Vec3) -> Option<(Pos2, f32)> {
        let c = self.vp * p.extend(1.0);
        if c.w < 0.05 {
            return None;
        }
        let ndc = c.xyz() / c.w;
        let x = self.rect.center().x + ndc.x * self.rect.width() * 0.5;
        let y = self.rect.center().y - ndc.y * self.rect.height() * 0.5;
        Some((Pos2::new(x, y), c.w))
    }

    /// World-space ray through a screen point.
    fn ray(&self, s: Pos2) -> (Vec3, Vec3) {
        let nx = (s.x - self.rect.center().x) / (self.rect.width() * 0.5);
        let ny = -(s.y - self.rect.center().y) / (self.rect.height() * 0.5);
        let near = self.inv.project_point3(Vec3::new(nx, ny, 0.0));
        let far = self.inv.project_point3(Vec3::new(nx, ny, 1.0));
        (near, (far - near).normalize_or(Vec3::NEG_Z))
    }

    /// World offset that moves a point at depth `at` by the screen delta `d`.
    fn screen_delta_to_world(&self, at: Vec3, d: Vec2) -> Vec3 {
        let Some((s, _)) = self.project(at) else {
            return Vec3::ZERO;
        };
        let (o1, d1) = self.ray(s);
        let (o2, d2) = self.ray(s + d);
        let depth = (at - o1).dot(d1);
        (o2 + d2 * depth) - (o1 + d1 * depth)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------------------------------------------

/// An interactive operation previewed live and confirmed with a click.
#[derive(Clone, Copy, PartialEq)]
enum ModalKind {
    Grab,
    Rotate,
    Scale,
    Extrude,
    Inset,
    Bevel,
}

struct Modal {
    kind: ModalKind,
    start: Pos2,
    /// Selection when the modal started (the preview is rebuilt from it every time the parameter changes).
    base: EditMesh,
    preview: Option<MeshCommand>,
    /// For extrude: the extrude command applied once; the modal then drags the cap.
    extrude: Option<MeshCommand>,
    pivot: Vec3,
    segments: u32,
    last_param: Option<(i32, i32, u32)>,
    value_text: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hover {
    None,
    Vert(VertId),
    Edge(EdgeId),
    Face(FaceId),
}

struct LoopCutTool {
    cuts: u32,
}

struct App {
    edit: EditMesh,
    history: History,
    camera: Camera,
    tris: Triangulated,
    tris_hash: u64,
    modal: Option<Modal>,
    loop_cut: Option<LoopCutTool>,
    hover: Hover,
    message: Option<(String, bool)>,
    xray: bool,
    symmetry: bool,
}

impl App {
    fn new() -> Self {
        let mesh = Primitive::Cube { size: 2.0 }
            .build(Vec3::ZERO)
            .unwrap_or_default();
        let mut app = Self {
            edit: EditMesh::new(mesh),
            history: History::new(),
            camera: Camera {
                yaw: 0.65,
                pitch: 0.48,
                distance: 9.0,
                target: Vec3::new(0.0, 0.3, 0.0),
            },
            tris: Triangulated::default(),
            tris_hash: 0,
            modal: None,
            loop_cut: None,
            hover: Hover::None,
            message: None,
            xray: false,
            symmetry: false,
        };
        app.refresh();
        app
    }

    /// A pre-edited scene for screenshots: a cube extruded, inset, bevelled and loop-cut.
    fn showcase(scene: &str) -> Self {
        let mut app = Self::new();
        let mesh = &app.edit.mesh;
        let top = mesh
            .faces()
            .map(|(f, _)| f)
            .max_by(|&a, &b| mesh.face_normal(a).y.total_cmp(&mesh.face_normal(b).y));
        if let Some(top) = top {
            let steps = [
                Op::Select(Selection::from_faces(mesh, [top])),
                Op::Inset {
                    thickness: 0.35,
                    depth: 0.0,
                    individual: false,
                },
                Op::ExtrudeFaces {
                    distance: 1.1,
                    individual: false,
                },
                Op::Inset {
                    thickness: 0.15,
                    depth: -0.25,
                    individual: false,
                },
            ];
            for op in steps {
                let _ = app.history.apply(&mut app.edit, &op);
            }
        }
        // Round the outer vertical edges of the base.
        let verticals: Vec<EdgeId> = app
            .edit
            .mesh
            .edges()
            .filter(|(_, e)| {
                let (a, b) = (
                    app.edit.mesh.position_or_zero(e.verts[0]),
                    app.edit.mesh.position_or_zero(e.verts[1]),
                );
                (a.x.abs() - 1.0).abs() < 1e-4
                    && (a.z.abs() - 1.0).abs() < 1e-4
                    && (a.x - b.x).abs() < 1e-4
                    && (a.z - b.z).abs() < 1e-4
            })
            .map(|(id, _)| id)
            .collect();
        let select = Op::Select(Selection::from_edges(&app.edit.mesh, verticals));
        let _ = app.history.apply(&mut app.edit, &select);
        let _ = app.history.apply(
            &mut app.edit,
            &Op::Bevel {
                width: 0.3,
                segments: 3,
            },
        );
        match scene {
            "edges" => {
                let mesh = &app.edit.mesh;
                let ring_edge = mesh.edges().map(|(id, _)| id).find(|&e| {
                    let m = mesh.edge_midpoint(e);
                    (m.y - 0.0).abs() < 1e-3 && m.z > 0.99 && m.x.abs() < 0.5
                });
                if let Some(e) = ring_edge {
                    let _ = app
                        .history
                        .apply(&mut app.edit, &Op::LoopCut { edge: e, cuts: 1 });
                }
                app.edit
                    .selection
                    .set_mode(SelectMode::Edge, &app.edit.mesh);
                app.message = Some((
                    "Loop cut: 1 cut. Alt+click an edge to select its loop.".into(),
                    false,
                ));
            }
            "verts" => {
                let mut sel = Selection::new(SelectMode::Vertex);
                sel.verts = app
                    .edit
                    .mesh
                    .verts()
                    .filter(|(_, v)| v.position.y > 2.0)
                    .map(|(id, _)| id)
                    .collect();
                sel.flush(&app.edit.mesh);
                let _ = app.history.apply(&mut app.edit, &Op::Select(sel));
                app.symmetry = true;
            }
            _ => {
                let mesh = &app.edit.mesh;
                let cap: Vec<FaceId> = mesh
                    .faces()
                    .map(|(f, _)| f)
                    .filter(|&f| mesh.face_center(f).y > 1.7 && mesh.face_normal(f).y > 0.9)
                    .collect();
                let select = Op::Select(Selection::from_faces(mesh, cap));
                let _ = app.history.apply(&mut app.edit, &select);
                app.message = Some((
                    "Inset (I) then extruded (E). Ctrl+Z steps back through every edit.".into(),
                    false,
                ));
            }
        }
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let h = self.edit.mesh.content_hash();
        if h != self.tris_hash || self.tris.triangle_faces.is_empty() {
            self.tris = triangulate(&self.edit.mesh, NormalMode::Flat);
            self.tris_hash = h;
        }
    }

    fn run(&mut self, op: Op) {
        match self.history.apply(&mut self.edit, &op) {
            Ok(()) => self.message = Some((format!("{} done.", op.label()), false)),
            Err(e) => self.message = Some((format!("{}: {e}", op.label()), true)),
        }
        self.refresh();
    }

    fn set_mode(&mut self, mode: SelectMode) {
        let mut sel = self.edit.selection.clone();
        sel.set_mode(mode, &self.edit.mesh);
        if sel != self.edit.selection {
            self.run(Op::Select(sel));
        }
    }

    fn mode(&self) -> SelectMode {
        self.edit.selection.mode
    }

    fn frame_all(&mut self) {
        if let Some((lo, hi)) = self.edit.mesh.bounds() {
            self.camera.target = (lo + hi) * 0.5;
            self.camera.distance = ((hi - lo).length() * 1.6).clamp(2.0, 200.0);
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // Modal tools
    // -----------------------------------------------------------------------------------------------------------

    fn start_modal(&mut self, kind: ModalKind, mouse: Pos2) {
        if self.edit.selection.verts.is_empty() {
            self.message = Some((
                "Select something first (click it, or press A for all).".into(),
                true,
            ));
            return;
        }
        let pivot = self
            .edit
            .selection
            .center(&self.edit.mesh)
            .unwrap_or(Vec3::ZERO);
        let mut extrude = None;
        if kind == ModalKind::Extrude {
            let op = match contextual_extrude(&self.edit.selection, 0.0, Vec3::ZERO) {
                Ok(op) => op,
                Err(e) => {
                    self.message = Some((e.to_string(), true));
                    return;
                }
            };
            if matches!(op, Op::ExtrudeEdges { .. } | Op::ExtrudeVerts { .. }) {
                // Edges and vertices: extrude in place, then the modal moves the new part with the mouse.
                match MeshCommand::run(&mut self.edit, &op) {
                    Ok(cmd) => extrude = Some(cmd),
                    Err(e) => {
                        self.message = Some((e.to_string(), true));
                        return;
                    }
                }
            }
        }
        if kind == ModalKind::Bevel && self.edit.selection.edges.is_empty() {
            self.message = Some((
                "Bevel works on edges: switch to edge mode (2) and select some.".into(),
                true,
            ));
            return;
        }
        if kind == ModalKind::Inset && self.edit.selection.faces.is_empty() {
            self.message = Some((
                "Inset works on faces: switch to face mode (3) and select some.".into(),
                true,
            ));
            return;
        }
        self.modal = Some(Modal {
            kind,
            start: mouse,
            base: self.edit.clone(),
            preview: None,
            extrude,
            pivot,
            segments: 1,
            last_param: None,
            value_text: String::new(),
        });
    }

    fn update_modal(&mut self, mouse: Pos2, proj: &Projector, wheel: f32) {
        let Some(mut m) = self.modal.take() else {
            return;
        };
        if wheel != 0.0 && m.kind == ModalKind::Bevel {
            m.segments = (m.segments as i32 + wheel.signum() as i32).clamp(1, 12) as u32;
        }
        let d = mouse - m.start;
        let pivot_screen = proj.project(m.pivot).map_or(m.start, |p| p.0);
        let scale_px = 0.01 * self.camera.distance / 8.0;
        let op = match m.kind {
            ModalKind::Grab => {
                let off = proj.screen_delta_to_world(m.pivot, d);
                m.value_text = format!("Move {:.2}, {:.2}, {:.2}", off.x, off.y, off.z);
                Some(Op::Transform(self.with_symmetry(Transform::translate(off))))
            }
            ModalKind::Rotate => {
                let a0 = (m.start - pivot_screen).angle();
                let a1 = (mouse - pivot_screen).angle();
                let axis = (proj.eye - m.pivot).normalize_or(Vec3::Y);
                let angle = -(a1 - a0);
                m.value_text = format!("Rotate {:.0}°", angle.to_degrees());
                Some(Op::Transform(self.with_symmetry(Transform::rotate(
                    Quat::from_axis_angle(axis, angle),
                    m.pivot,
                ))))
            }
            ModalKind::Scale => {
                let r0 = (m.start - pivot_screen).length().max(4.0);
                let k = (mouse - pivot_screen).length() / r0;
                m.value_text = format!("Scale {k:.2}");
                Some(Op::Transform(
                    self.with_symmetry(Transform::scale(Vec3::splat(k), m.pivot)),
                ))
            }
            ModalKind::Extrude => {
                if m.extrude.is_some() {
                    let off = proj.screen_delta_to_world(m.pivot, d);
                    m.value_text = format!("Extrude {:.2}", off.length());
                    Some(Op::Transform(Transform::translate(off)))
                } else {
                    let dist = -d.y * scale_px;
                    m.value_text = format!("Extrude {dist:.2}");
                    Some(Op::ExtrudeFaces {
                        distance: dist,
                        individual: false,
                    })
                }
            }
            ModalKind::Inset => {
                let t = ((mouse - pivot_screen).length() - (m.start - pivot_screen).length()).abs()
                    * scale_px
                    * 0.5;
                m.value_text = format!("Inset {t:.2}");
                Some(Op::Inset {
                    thickness: t,
                    depth: 0.0,
                    individual: false,
                })
            }
            ModalKind::Bevel => {
                let w = ((mouse - pivot_screen).length() - (m.start - pivot_screen).length())
                    .max(0.0)
                    * scale_px
                    * 0.5;
                m.value_text = format!(
                    "Bevel {w:.2} · {} segment{}",
                    m.segments,
                    if m.segments == 1 { "" } else { "s" }
                );
                if w > 1e-3 {
                    Some(Op::Bevel {
                        width: w,
                        segments: m.segments,
                    })
                } else {
                    None
                }
            }
        };
        let key = ((d.x * 2.0) as i32, (d.y * 2.0) as i32, m.segments);
        if m.last_param != Some(key) {
            m.last_param = Some(key);
            if let Some(prev) = m.preview.take() {
                prev.undo(&mut self.edit);
            }
            if let Some(op) = op {
                match MeshCommand::run(&mut self.edit, &op) {
                    Ok(cmd) => m.preview = Some(cmd),
                    Err(e) => m.value_text = format!("{} ({e})", m.value_text),
                }
            }
            self.refresh();
        }
        self.modal = Some(m);
    }

    fn with_symmetry(&self, xf: Transform) -> Transform {
        if self.symmetry {
            xf.with_symmetry(Symmetry::default())
        } else {
            xf
        }
    }

    fn finish_modal(&mut self, confirm: bool) {
        let Some(m) = self.modal.take() else { return };
        if confirm {
            let label = match m.kind {
                ModalKind::Grab => "Move",
                ModalKind::Rotate => "Rotate",
                ModalKind::Scale => "Scale",
                ModalKind::Extrude => "Extrude",
                ModalKind::Inset => "Inset",
                ModalKind::Bevel => "Bevel",
            };
            if let Some(ex) = m.extrude {
                self.history.push(ex);
            }
            if let Some(p) = m.preview {
                self.history.push(p);
            }
            self.message = Some((format!("{label} done. Ctrl+Z to undo."), false));
        } else {
            if let Some(p) = m.preview {
                p.undo(&mut self.edit);
            }
            if let Some(ex) = m.extrude {
                ex.undo(&mut self.edit);
            }
            self.edit = m.base;
            self.message = Some(("Cancelled.".into(), false));
        }
        self.refresh();
    }

    // -----------------------------------------------------------------------------------------------------------
    // Picking
    // -----------------------------------------------------------------------------------------------------------

    fn front_facing(&self, f: FaceId, eye: Vec3) -> bool {
        let n = self.edit.mesh.face_normal(f);
        n.dot(eye - self.edit.mesh.face_center(f)) > 0.0
    }

    fn vert_visible(&self, v: VertId, eye: Vec3) -> bool {
        let faces = self.edit.mesh.vert_faces(v);
        self.xray || faces.is_empty() || faces.iter().any(|&f| self.front_facing(f, eye))
    }

    fn edge_visible(&self, e: EdgeId, eye: Vec3) -> bool {
        let faces = self.edit.mesh.edge_faces(e);
        self.xray || faces.is_empty() || faces.iter().any(|&f| self.front_facing(f, eye))
    }

    fn pick_face(&self, proj: &Projector, s: Pos2) -> Option<FaceId> {
        let (o, d) = proj.ray(s);
        let data = &self.tris.data;
        let mut best: Option<(f32, FaceId)> = None;
        for (t, tri) in data.indices.as_chunks::<3>().0.iter().enumerate() {
            let [a, b, c] =
                [tri[0], tri[1], tri[2]].map(|i| Vec3::from_array(data.positions[i as usize]));
            if let Some(dist) = ray_triangle(o, d, a, b, c)
                && best.is_none_or(|(bd, _)| dist < bd)
            {
                best = Some((dist, self.tris.triangle_faces[t]));
            }
        }
        best.map(|b| b.1)
    }

    fn pick(&self, proj: &Projector, s: Pos2) -> Hover {
        let mesh = &self.edit.mesh;
        match self.mode() {
            SelectMode::Face => self.pick_face(proj, s).map_or(Hover::None, Hover::Face),
            SelectMode::Vertex => {
                let mut best: Option<(f32, VertId)> = None;
                for (v, vert) in mesh.verts() {
                    let Some((p, _)) = proj.project(vert.position) else {
                        continue;
                    };
                    let dist = p.distance(s);
                    if dist < 14.0
                        && best.is_none_or(|(bd, _)| dist < bd)
                        && self.vert_visible(v, proj.eye)
                    {
                        best = Some((dist, v));
                    }
                }
                best.map_or(Hover::None, |b| Hover::Vert(b.1))
            }
            SelectMode::Edge => self.pick_edge(proj, s).map_or(Hover::None, Hover::Edge),
        }
    }

    fn pick_edge(&self, proj: &Projector, s: Pos2) -> Option<EdgeId> {
        let mesh = &self.edit.mesh;
        let mut best: Option<(f32, EdgeId)> = None;
        for (e, edge) in mesh.edges() {
            let (Some((a, _)), Some((b, _))) = (
                proj.project(mesh.position_or_zero(edge.verts[0])),
                proj.project(mesh.position_or_zero(edge.verts[1])),
            ) else {
                continue;
            };
            let dist = segment_distance(s, a, b);
            if dist < 10.0 && best.is_none_or(|(bd, _)| dist < bd) && self.edge_visible(e, proj.eye)
            {
                best = Some((dist, e));
            }
        }
        best.map(|b| b.1)
    }

    fn click(&mut self, hover: Hover, shift: bool, alt: bool, ctrl: bool) {
        let mesh = &self.edit.mesh;
        let mut sel = if shift {
            self.edit.selection.clone()
        } else {
            Selection::new(self.mode())
        };
        sel.mode = self.mode();
        match hover {
            Hover::None => {
                if shift {
                    return;
                }
            }
            Hover::Vert(v) => sel.toggle_vert(mesh, v),
            Hover::Edge(e) => {
                if alt {
                    let edges: Vec<EdgeId> = if ctrl {
                        edge_ring(mesh, e).edges
                    } else {
                        edge_loop(mesh, e)
                    };
                    for x in edges {
                        sel.edges.insert(x);
                    }
                    sel.flush(mesh);
                } else {
                    sel.toggle_edge(mesh, e);
                }
            }
            Hover::Face(f) => sel.toggle_face(mesh, f),
        }
        if sel != self.edit.selection {
            self.run(Op::Select(sel));
            self.message = None;
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // UI
    // -----------------------------------------------------------------------------------------------------------

    fn shortcuts(&mut self, ui: &egui::Ui, mouse: Option<Pos2>) {
        let (pressed, mods) = ui.input(|i| {
            let keys = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::A,
                Key::G,
                Key::R,
                Key::S,
                Key::E,
                Key::I,
                Key::B,
                Key::F,
                Key::M,
                Key::X,
                Key::Delete,
                Key::Z,
                Key::Y,
                Key::Home,
                Key::Escape,
                Key::Enter,
            ];
            (
                keys.iter()
                    .copied()
                    .filter(|&k| i.key_pressed(k))
                    .collect::<Vec<_>>(),
                i.modifiers,
            )
        });
        let mouse = mouse.unwrap_or_default();
        let ctrl = mods.command;
        for key in pressed {
            if self.modal.is_some() {
                match key {
                    Key::Escape => self.finish_modal(false),
                    Key::Enter => self.finish_modal(true),
                    _ => {}
                }
                continue;
            }
            if self.loop_cut.is_some() && key == Key::Escape {
                self.loop_cut = None;
                continue;
            }
            match key {
                Key::Num1 => self.set_mode(SelectMode::Vertex),
                Key::Num2 => self.set_mode(SelectMode::Edge),
                Key::Num3 => self.set_mode(SelectMode::Face),
                Key::A if mods.alt => self.run(Op::Select(Selection::new(self.mode()))),
                Key::A => {
                    let mut s = Selection::new(self.mode());
                    s.select_all(&self.edit.mesh);
                    self.run(Op::Select(s));
                }
                Key::Z if ctrl && mods.shift => self.redo(),
                Key::Z if ctrl => self.undo(),
                Key::Z if mods.alt => self.xray = !self.xray,
                Key::Y if ctrl => self.redo(),
                Key::G => self.start_modal(ModalKind::Grab, mouse),
                Key::R if ctrl => self.loop_cut = Some(LoopCutTool { cuts: 1 }),
                Key::R => self.start_modal(ModalKind::Rotate, mouse),
                Key::S => self.start_modal(ModalKind::Scale, mouse),
                Key::E => self.start_modal(ModalKind::Extrude, mouse),
                Key::I => self.start_modal(ModalKind::Inset, mouse),
                Key::B if ctrl => self.start_modal(ModalKind::Bevel, mouse),
                Key::F => self.run(Op::Fill),
                Key::M => self.run(Op::MergeAtCenter),
                Key::X | Key::Delete => self.delete(),
                Key::Home => self.frame_all(),
                _ => {}
            }
        }
    }

    fn delete(&mut self) {
        let kind = match self.mode() {
            SelectMode::Vertex => DeleteKind::Verts,
            SelectMode::Edge => DeleteKind::Edges,
            SelectMode::Face => DeleteKind::Faces,
        };
        self.run(Op::Delete(kind));
    }

    fn undo(&mut self) {
        match self.history.undo(&mut self.edit) {
            Some(l) => self.message = Some((format!("Undid {l}."), false)),
            None => self.message = Some(("Nothing to undo.".into(), false)),
        }
        self.refresh();
    }

    fn redo(&mut self) {
        match self.history.redo(&mut self.edit) {
            Some(l) => self.message = Some((format!("Redid {l}."), false)),
            None => self.message = Some(("Nothing to redo.".into(), false)),
        }
        self.refresh();
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let section = |ui: &mut egui::Ui, title: &str| {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(title.to_uppercase())
                    .size(10.5)
                    .color(TEXT_3),
            );
            ui.add_space(2.0);
        };
        let tool = |ui: &mut egui::Ui, label: &str, key: &str| -> bool {
            ui.add_sized(
                [ui.available_width(), 26.0],
                egui::Button::new(egui::RichText::new(label).color(TEXT))
                    .shortcut_text(egui::RichText::new(key).color(TEXT_3).size(11.5)),
            )
            .clicked()
        };
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Modeller")
                .font(semibold(15.0))
                .color(TEXT),
        );
        section(ui, "Select");
        ui.horizontal(|ui| {
            for (mode, label) in [
                (SelectMode::Vertex, "Vertex"),
                (SelectMode::Edge, "Edge"),
                (SelectMode::Face, "Face"),
            ] {
                let on = self.mode() == mode;
                let text = egui::RichText::new(label).color(if on { TEXT } else { TEXT_2 });
                let b = egui::Button::new(text)
                    .selected(on)
                    .min_size(Vec2::new(58.0, 26.0));
                if ui.add(b).clicked() {
                    self.set_mode(mode);
                }
            }
        });
        let mut ops: Vec<Op> = Vec::new();
        let mut modal: Option<ModalKind> = None;
        section(ui, "Shape");
        if tool(ui, "Extrude", "E") {
            modal = Some(ModalKind::Extrude);
        }
        if tool(ui, "Inset", "I") {
            modal = Some(ModalKind::Inset);
        }
        if tool(ui, "Bevel", "Ctrl B") {
            modal = Some(ModalKind::Bevel);
        }
        if tool(ui, "Loop cut", "Ctrl R") {
            self.loop_cut = Some(LoopCutTool { cuts: 1 });
        }
        if tool(ui, "Fill", "F") {
            ops.push(Op::Fill);
        }
        if tool(ui, "Merge at centre", "M") {
            ops.push(Op::MergeAtCenter);
        }
        if tool(ui, "Bridge loops", "") {
            ops.push(Op::BridgeEdgeLoops);
        }
        if tool(ui, "Delete", "X") {
            self.delete();
        }
        section(ui, "Surface");
        if tool(ui, "Subdivide", "") {
            ops.push(Op::Subdivide { levels: 1 });
        }
        if tool(ui, "Smooth (Catmull-Clark)", "") {
            ops.push(Op::CatmullClark { levels: 1 });
        }
        if tool(ui, "Mirror X", "") {
            ops.push(Op::Mirror {
                axis: metallic_modeling::Axis::X,
                merge_distance: 1e-3,
            });
        }
        if tool(ui, "Recalculate outside", "") {
            ops.push(Op::RecalculateOutside);
        }
        if tool(ui, "Mark sharp", "") {
            ops.push(Op::MarkSharp(true));
        }
        if tool(ui, "Mark seam", "") {
            ops.push(Op::MarkSeam(true));
        }
        if tool(ui, "Unwrap UVs", "") {
            ops.push(Op::UvUnwrap(metallic_modeling::uv::UnwrapOptions::default()));
        }
        section(ui, "View");
        ui.checkbox(
            &mut self.symmetry,
            egui::RichText::new("Mirror edits across X").color(TEXT_2),
        );
        ui.checkbox(
            &mut self.xray,
            egui::RichText::new("X-ray (Alt Z)").color(TEXT_2),
        );
        section(ui, "Start over");
        ui.horizontal(|ui| {
            for (label, p) in [
                ("Cube", Primitive::Cube { size: 2.0 }),
                (
                    "Cylinder",
                    Primitive::Cylinder {
                        segments: 16,
                        radius: 1.0,
                        height: 2.0,
                    },
                ),
                (
                    "Sphere",
                    Primitive::UvSphere {
                        segments: 16,
                        rings: 10,
                        radius: 1.2,
                    },
                ),
            ] {
                if ui
                    .button(egui::RichText::new(label).color(TEXT_2))
                    .clicked()
                {
                    let mut all = Selection::new(self.mode());
                    all.select_all(&self.edit.mesh);
                    ops.push(Op::Select(all));
                    ops.push(Op::Delete(DeleteKind::Verts));
                    ops.push(Op::AddPrimitive {
                        primitive: p,
                        center: Vec3::ZERO,
                    });
                }
            }
        });
        for op in ops {
            self.run(op);
        }
        if let Some(kind) = modal {
            let center = ui.ctx().content_rect().center();
            self.start_modal(kind, center);
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let mesh = &self.edit.mesh;
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Mesh").font(semibold(15.0)).color(TEXT));
        ui.add_space(4.0);
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(k).color(TEXT_2));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(v)
                            .family(FontFamily::Monospace)
                            .color(TEXT),
                    );
                });
            });
        };
        row(ui, "Vertices", mesh.vert_count().to_string());
        row(ui, "Edges", mesh.edge_count().to_string());
        row(ui, "Faces", mesh.face_count().to_string());
        row(ui, "Triangles", self.tris.data.triangle_count().to_string());
        let closed = mesh.is_closed_manifold();
        row(
            ui,
            "Watertight",
            if closed { "yes".into() } else { "no".into() },
        );
        let s = &self.edit.selection;
        row(
            ui,
            "Selected",
            format!("{} / {} / {}", s.verts.len(), s.edges.len(), s.faces.len()),
        );
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("History")
                .font(semibold(15.0))
                .color(TEXT),
        );
        ui.add_space(4.0);
        let labels: Vec<String> = self
            .history
            .labels()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let n = labels.len();
                for (i, l) in labels.iter().enumerate() {
                    let last = i + 1 == n;
                    ui.label(egui::RichText::new(l).color(if last { TEXT } else { TEXT_3 }));
                }
                if labels.is_empty() {
                    ui.label(egui::RichText::new("Your edits appear here.").color(TEXT_3));
                }
            });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let hint = if let Some(m) = &self.modal {
            let what = match m.kind {
                ModalKind::Bevel => "Move the mouse to set the width · wheel changes segments",
                ModalKind::Extrude => "Move the mouse to pull the new part out",
                ModalKind::Inset => "Move the mouse to set the inset",
                ModalKind::Grab => "Move the mouse",
                ModalKind::Rotate => "Circle the mouse around the selection",
                ModalKind::Scale => "Move the mouse away from or towards the selection",
            };
            format!("{what} · click or Enter to apply · Esc or right-click to cancel")
        } else if let Some(t) = &self.loop_cut {
            format!(
                "Loop cut: point at an edge · wheel changes cuts ({}) · click to cut · Esc to stop",
                t.cuts
            )
        } else {
            match self.mode() {
                SelectMode::Vertex => "Vertex mode · click to select, Shift+click to add · G move · E extrude · M merge · F fill",
                SelectMode::Edge => "Edge mode · Alt+click selects a loop · Ctrl+B bevel · Ctrl+R loop cut · E extrude",
                SelectMode::Face => "Face mode · click to select · E extrude · I inset · X delete · Ctrl+Z undo",
            }
            .to_owned()
        };
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(hint).color(TEXT_2).size(12.5));
            if let Some((msg, err)) = &self.message {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(msg)
                        .color(if *err { ERROR } else { TEXT_3 })
                        .size(12.5),
                );
            }
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let (mods, wheel, mouse, middle, primary_down) = ui.input(|i| {
            (
                i.modifiers,
                i.smooth_scroll_delta.y,
                i.pointer.hover_pos(),
                i.pointer.middle_down(),
                i.pointer.primary_down(),
            )
        });
        let in_view = mouse.is_some_and(|m| rect.contains(m));
        // Camera.
        let drag = response.drag_delta();
        let orbit = (middle && !mods.shift)
            || (primary_down && mods.alt && self.modal.is_none() && response.dragged());
        if orbit && in_view {
            self.camera.yaw -= drag.x * 0.008;
            self.camera.pitch = (self.camera.pitch + drag.y * 0.008).clamp(-1.5, 1.5);
            let d = ui.input(|i| i.pointer.delta());
            if !response.dragged() {
                self.camera.yaw -= d.x * 0.008;
                self.camera.pitch = (self.camera.pitch + d.y * 0.008).clamp(-1.5, 1.5);
            }
        }
        if middle && mods.shift && in_view {
            let d = ui.input(|i| i.pointer.delta());
            let proj = Projector::new(&self.camera, rect);
            self.camera.target -= proj.screen_delta_to_world(self.camera.target, d);
        }
        let wheel_free = self.modal.is_none() && self.loop_cut.is_none();
        if in_view && wheel != 0.0 && wheel_free {
            self.camera.distance =
                (self.camera.distance * (1.0 - wheel * 0.0015)).clamp(1.0, 300.0);
        }
        let proj = Projector::new(&self.camera, rect);
        // Modal tools.
        if self.modal.is_some() {
            if let Some(m) = mouse {
                self.update_modal(m, &proj, if in_view { wheel } else { 0.0 });
            }
            if response.clicked() {
                self.finish_modal(true);
            } else if response.secondary_clicked() {
                self.finish_modal(false);
            }
        }
        // Hover and selection.
        self.hover = match (mouse, in_view, self.modal.is_some()) {
            (Some(m), true, false) => {
                if self.loop_cut.is_some() {
                    self.pick_edge(&proj, m).map_or(Hover::None, Hover::Edge)
                } else {
                    self.pick(&proj, m)
                }
            }
            _ => Hover::None,
        };
        if let Some(tool) = self.loop_cut.as_mut() {
            if in_view && wheel != 0.0 {
                tool.cuts = (tool.cuts as i32 + wheel.signum() as i32).clamp(1, 16) as u32;
            }
            if response.clicked() {
                if let Hover::Edge(e) = self.hover {
                    let cuts = tool.cuts;
                    self.loop_cut = None;
                    self.run(Op::LoopCut { edge: e, cuts });
                }
            } else if response.secondary_clicked() {
                self.loop_cut = None;
            }
        } else if self.modal.is_none() && response.clicked() && !mods.alt
            || (response.clicked()
                && mods.alt
                && matches!(self.hover, Hover::Edge(_))
                && self.modal.is_none())
        {
            self.click(self.hover, mods.shift, mods.alt, mods.command);
        }
        self.paint(ui, rect, &proj);
    }

    fn paint(&self, ui: &egui::Ui, rect: Rect, proj: &Projector) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, BG_BASE);
        // Floor grid.
        for i in -10..=10 {
            let f = i as f32;
            let col = if i == 0 { LINE_STRONG } else { LINE };
            for (a, b) in [
                (Vec3::new(f, 0.0, -10.0), Vec3::new(f, 0.0, 10.0)),
                (Vec3::new(-10.0, 0.0, f), Vec3::new(10.0, 0.0, f)),
            ] {
                if let (Some((pa, _)), Some((pb, _))) = (proj.project(a), proj.project(b)) {
                    painter.line_segment([pa, pb], Stroke::new(1.0, col));
                }
            }
        }
        let mesh = &self.edit.mesh;
        let sel = &self.edit.selection;
        let light =
            (proj.eye - self.camera.target).normalize_or(Vec3::Y) + Vec3::new(0.3, 0.8, 0.2);
        let light = light.normalize();
        // Faces, back to front.
        let data = &self.tris.data;
        let mut order: Vec<(f32, usize)> = Vec::with_capacity(self.tris.triangle_faces.len());
        let mut screen: Vec<Option<(Pos2, f32)>> = data
            .positions
            .iter()
            .map(|p| proj.project(Vec3::from_array(*p)))
            .collect();
        for (t, tri) in data.indices.as_chunks::<3>().0.iter().enumerate() {
            let pts: Vec<_> = tri.iter().map(|&i| screen[i as usize]).collect();
            if pts.iter().all(Option::is_some) {
                let depth = pts.iter().map(|p| p.map_or(0.0, |x| x.1)).sum::<f32>() / 3.0;
                order.push((depth, t));
            }
        }
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut egui_mesh = egui::Mesh::default();
        for (_, t) in order {
            let f = self.tris.triangle_faces[t];
            let ids = &data.indices[t * 3..t * 3 + 3];
            let n = Vec3::from_array(data.normals[ids[0] as usize]);
            let center = Vec3::from_array(data.positions[ids[0] as usize]);
            let facing = n.dot(proj.eye - center) > 0.0;
            let lambert = n.dot(light).abs() * 0.75 + 0.25;
            let mut rgb = SURFACE.map(|c| c * lambert);
            if !facing {
                rgb = rgb.map(|c| c * 0.45);
            }
            let selected = sel.faces.contains(&f);
            let hovered = self.hover == Hover::Face(f);
            if selected {
                let a = [0.616, 0.545, 0.878];
                rgb = [0, 1, 2].map(|k| rgb[k] * 0.55 + a[k] * 0.45 * lambert.max(0.6));
            } else if hovered {
                rgb = rgb.map(|c| (c * 1.15).min(1.0));
            }
            let alpha = if self.xray { 150 } else { 255 };
            let col = Color32::from_rgba_unmultiplied(
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8,
                alpha,
            );
            let base = egui_mesh.vertices.len() as u32;
            for &i in ids {
                if let Some((p, _)) = screen[i as usize] {
                    egui_mesh.colored_vertex(p, col);
                }
            }
            egui_mesh.add_triangle(base, base + 1, base + 2);
        }
        painter.add(egui::Shape::mesh(egui_mesh));
        screen.clear();
        // Edges.
        let ring_preview: BTreeSet<EdgeId> = match (&self.loop_cut, self.hover) {
            (Some(_), Hover::Edge(e)) => edge_ring(mesh, e).edges.into_iter().collect(),
            _ => BTreeSet::new(),
        };
        for (e, edge) in mesh.edges() {
            if !self.edge_visible(e, proj.eye) {
                continue;
            }
            let (Some((a, _)), Some((b, _))) = (
                proj.project(mesh.position_or_zero(edge.verts[0])),
                proj.project(mesh.position_or_zero(edge.verts[1])),
            ) else {
                continue;
            };
            let (w, col) = if sel.edges.contains(&e) && self.mode() != SelectMode::Face {
                (2.0, ACCENT)
            } else if self.hover == Hover::Edge(e) {
                (2.0, ACCENT_TEXT)
            } else if edge.flags.seam {
                (1.6, WARNING)
            } else if edge.flags.sharp {
                (1.6, INFO)
            } else if sel.edges.contains(&e) {
                (1.2, hex(0x6d5fa8))
            } else {
                (1.0, hex(0x0d0c10))
            };
            painter.line_segment([a, b], Stroke::new(w, col));
            if ring_preview.contains(&e) {
                // Preview where the new loop(s) will run: points across each ring edge.
                let cuts = self.loop_cut.as_ref().map_or(1, |t| t.cuts);
                for k in 1..=cuts {
                    let t = k as f32 / (cuts + 1) as f32;
                    painter.circle_filled(a + (b - a) * t, 2.5, WARNING);
                }
            }
        }
        // Loop-cut preview lines across the ring's quads.
        if let (Some(tool), Hover::Edge(e)) = (&self.loop_cut, self.hover) {
            let ring = edge_ring(mesh, e);
            let mut pairs: Vec<(EdgeId, EdgeId)> =
                ring.edges.windows(2).map(|w| (w[0], w[1])).collect();
            if ring.closed && ring.edges.len() > 1 {
                pairs.push((ring.edges[ring.edges.len() - 1], ring.edges[0]));
            }
            for (e0, e1) in pairs {
                for k in 1..=tool.cuts {
                    let t = k as f32 / (tool.cuts + 1) as f32;
                    if let (Some(pa), Some(pb)) =
                        (edge_point(mesh, e0, e1, t), edge_point(mesh, e1, e0, t))
                        && let (Some((sa, _)), Some((sb, _))) = (proj.project(pa), proj.project(pb))
                    {
                        painter.line_segment([sa, sb], Stroke::new(1.5, WARNING));
                    }
                }
            }
        }
        // Vertices.
        if self.mode() == SelectMode::Vertex {
            for (v, vert) in mesh.verts() {
                if !self.vert_visible(v, proj.eye) {
                    continue;
                }
                let Some((p, _)) = proj.project(vert.position) else {
                    continue;
                };
                if sel.verts.contains(&v) {
                    painter.circle_filled(p, 3.5, ACCENT);
                } else if self.hover == Hover::Vert(v) {
                    painter.circle_filled(p, 3.5, ACCENT_TEXT);
                } else {
                    painter.circle_filled(p, 2.2, ICON);
                }
            }
        }
        // Axis gizmo.
        let origin = Pos2::new(rect.left() + 46.0, rect.bottom() - 46.0);
        let view = glam::camera::rh::view::look_at_mat4(proj.eye, self.camera.target, Vec3::Y);
        let mut axes = [
            (Vec3::X, AXIS_X, "X"),
            (Vec3::Y, AXIS_Y, "Y"),
            (Vec3::Z, AXIS_Z, "Z"),
        ];
        axes.sort_by(|a, b| {
            view.transform_vector3(a.0)
                .z
                .total_cmp(&view.transform_vector3(b.0).z)
        });
        painter.circle_filled(origin, 30.0, BG_PANEL.gamma_multiply(0.9));
        for (axis, col, label) in axes {
            let d = view.transform_vector3(axis);
            let tip = origin + Vec2::new(d.x, -d.y) * 22.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, col));
            painter.circle_filled(tip, 7.5, col);
            painter.text(tip, Align2::CENTER_CENTER, label, semibold(10.5), BG_DEEP);
        }
        // Mode badge and modal value.
        let badge = match self.mode() {
            SelectMode::Vertex => "Vertex mode",
            SelectMode::Edge => "Edge mode",
            SelectMode::Face => "Face mode",
        };
        let galley_pos = Pos2::new(rect.left() + 14.0, rect.top() + 12.0);
        let r = painter.text(galley_pos, Align2::LEFT_TOP, badge, semibold(13.0), TEXT);
        if self.symmetry {
            painter.text(
                Pos2::new(r.right() + 10.0, r.top()),
                Align2::LEFT_TOP,
                "Mirror X on",
                FontId::proportional(12.5),
                ACCENT_TEXT,
            );
        }
        if let Some(m) = &self.modal
            && let Some(mp) = ui.input(|i| i.pointer.hover_pos())
        {
            let text_rect = painter.text(
                mp + Vec2::new(16.0, 14.0),
                Align2::LEFT_TOP,
                &m.value_text,
                FontId::monospace(12.5),
                TEXT,
            );
            painter.rect_stroke(
                text_rect.expand(4.0),
                CornerRadius::same(3),
                Stroke::new(1.0, LINE_STRONG),
                egui::StrokeKind::Outside,
            );
        }
    }
}

/// The point at fraction `t` along ring edge `e`, oriented consistently with its neighbour `other` in the ring.
fn edge_point(mesh: &metallic_modeling::Mesh, e: EdgeId, other: EdgeId, t: f32) -> Option<Vec3> {
    let edge = mesh.edge(e)?;
    let o = mesh.edge(other)?;
    let (a, b) = (
        mesh.position_or_zero(edge.verts[0]),
        mesh.position_or_zero(edge.verts[1]),
    );
    let (oa, ob) = (
        mesh.position_or_zero(o.verts[0]),
        mesh.position_or_zero(o.verts[1]),
    );
    // Orient both edges so their start points are on the same side of the quad.
    let (a, b) = if a.distance(oa).min(a.distance(ob)) <= b.distance(oa).min(b.distance(ob)) {
        (a, b)
    } else {
        (b, a)
    };
    Some(a.lerp(b, t))
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-4).then_some(t)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mouse = ui.input(|i| i.pointer.hover_pos());
        self.shortcuts(ui, mouse);
        egui::Panel::bottom("status")
            .exact_size(28.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_DEEP)
                    .inner_margin(Margin::symmetric(6, 0)),
            )
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tools")
            .exact_size(206.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(BG_PANEL)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("info")
            .exact_size(200.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(BG_PANEL)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
        if self.modal.is_some() || self.loop_cut.is_some() {
            ui.ctx().request_repaint();
        }
    }
}

fn setup(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    load_fonts(ctx);
    apply_theme(ctx);
}

fn screenshot(path: &str, scene: &str) -> Result<(), String> {
    let mut app = Some(App::showcase(scene));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_eframe(|cc| {
            setup(&cc.egui_ctx);
            app.take().unwrap_or_else(App::new)
        });
    // Hover something so the hover styling shows too.
    harness.run_steps(4);
    let image = harness.render()?;
    image.save(path).map_err(|e| e.to_string())?;
    println!("wrote {path}");
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--screenshot") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "modeling_demo.png".into());
        let scene = args.get(i + 2).cloned().unwrap_or_else(|| "faces".into());
        return screenshot(&path, &scene);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metallic modeller demo")
            .with_inner_size([1440.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Metallic modeller demo",
        options,
        Box::new(|cc| {
            setup(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
    .map_err(|e| e.to_string())
}
