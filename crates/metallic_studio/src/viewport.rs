//! The 3D viewport: draws the edit world with `metallic_render` into a texture egui shows, and
//! handles camera navigation, picking, selection and gizmo drags.
//!
//! - **Rendering** shares eframe's wgpu device. The world is drawn only when the world, the
//!   camera or the panel size changed, so an idle editor costs no GPU time (#124).
//! - **Selection**: click selects the part under the cursor, Model first (a second click on the
//!   same part selects the part itself); Ctrl-click toggles; Alt-click cycles through everything
//!   under the cursor; dragging on empty space draws a selection box.
//! - **Gizmos** ([`crate::gizmo`]): the active tool's handles sit at the selection's pivot. A drag
//!   previews live by writing the world and records one undo step on release; Esc or a right
//!   click during the drag cancels it.
//! - **Play**: while a session with a [`PlayView`](crate::play::PlayView) runs, the viewport draws
//!   that game every frame with the game camera, character and ScreenGuis instead of the edit
//!   world, framed by a thin `success` line. Picking, gizmos and the fly camera are off; the
//!   editor sends keyboard and mouse to the game ([`crate::play::input`]). After Stop the edit
//!   world is drawn again from the editor camera, which Play never moved.
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu;
use metallic_core::InstanceId;
use metallic_render::glam::{DQuat, DVec3};
use metallic_render::{RenderContext, Renderer};

use crate::camera::{EditorCamera, ViewProjection};
use crate::editor::{EditorState, Space, Tool, UiRequest};
use crate::gizmo::{self, DragModifiers, GizmoDrag, GizmoFrame, Handle};
use crate::picking::{Hit, PickCache};
use crate::theme;
use crate::tokens::ui_tokens;
use crate::transform::{Aabb, PartTransform, from_d};

/// Pointer travel (points) before a press becomes a drag.
const DRAG_SLOP: f32 = 4.0;
/// Most selection outlines drawn (beyond that the status bar still counts them).
const MAX_OUTLINES: usize = 2000;
/// Colour format of the viewport texture (egui samples it like its own textures).
const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

struct Gpu {
    renderer: Renderer,
    device: wgpu::Device,
    egui: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    target: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    texture_id: Option<egui::TextureId>,
}

impl Gpu {
    fn new(rs: &egui_wgpu::RenderState) -> Gpu {
        let ctx =
            RenderContext::from_eframe(rs.device.clone(), rs.queue.clone(), rs.adapter.clone());
        Gpu {
            renderer: Renderer::new(&ctx, TEXTURE_FORMAT),
            device: rs.device.clone(),
            egui: rs.renderer.clone(),
            target: None,
            texture_id: None,
        }
    }

    fn ensure_target(&mut self, size: (u32, u32)) {
        if self.target.as_ref().is_some_and(|t| t.2 == size) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio viewport"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut egui = self.egui.write();
        match self.texture_id {
            Some(id) => egui.update_egui_texture_from_wgpu_texture(
                &self.device,
                &view,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture_id = Some(egui.register_native_texture(
                    &self.device,
                    &view,
                    wgpu::FilterMode::Linear,
                ));
            }
        }
        drop(egui);
        self.target = Some((texture, view, size));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Press {
    /// Left press on empty space or a part, not yet a drag.
    Pending { at: Pos2 },
    /// Box selection from `from`.
    Box { from: Pos2 },
    /// Alt held on press: a click cycles selection, a drag orbits the camera.
    Alt { at: Pos2 },
    /// Nothing (gizmo drag, camera move, or released).
    None,
}

/// Viewport state.
pub struct Viewport {
    /// The editor camera.
    pub camera: EditorCamera,
    picks: PickCache,
    gpu: Option<Gpu>,
    hovered_part: Option<InstanceId>,
    hovered_handle: Option<Handle>,
    drag: Option<GizmoDrag>,
    press: Press,
    last_click: Option<InstanceId>,
    cycle: usize,
    hits: Vec<Hit>,
    rendered: Option<(metallic_core::ChangeCursor, EditorCamera, (u32, u32))>,
    gpu_missing_logged: bool,
    last_rect: Rect,
    /// Ray pick under the pointer, reused while pointer, world and camera are unchanged, so an
    /// idle pointer does not scan every part each frame.
    hover_pick: Option<(HoverKey, Option<Hit>)>,
    /// The pointer was over the viewport in the last play frame (input goes to the game).
    play_hovered: bool,
    /// The last frame drew a play session (the renderer holds the play world and avatar).
    showing_play: bool,
}

type HoverKey = (Pos2, metallic_core::ChangeCursor, EditorCamera, Rect);

impl Default for Viewport {
    fn default() -> Self {
        Viewport::new()
    }
}

impl Viewport {
    /// A viewport with the default camera; the GPU side starts on the first frame.
    pub fn new() -> Viewport {
        Viewport {
            camera: EditorCamera::default(),
            picks: PickCache::new(),
            gpu: None,
            hovered_part: None,
            hovered_handle: None,
            drag: None,
            press: Press::None,
            last_click: None,
            cycle: 0,
            hits: Vec::new(),
            rendered: None,
            gpu_missing_logged: false,
            last_rect: Rect::NOTHING,
            hover_pick: None,
            play_hovered: false,
            showing_play: false,
        }
    }

    /// True when the pointer was over the viewport in the last play frame.
    pub fn play_hovered(&self) -> bool {
        self.showing_play && self.play_hovered
    }

    /// Cancels a gizmo drag in progress, putting the parts back (Play starting).
    pub fn cancel_drag(&mut self, state: &mut EditorState) {
        if let Some(drag) = self.drag.take() {
            drag.cancel(&mut state.world);
        }
        self.press = Press::None;
    }

    /// The part under the cursor last frame.
    pub fn hovered(&self) -> Option<InstanceId> {
        self.hovered_part
    }

    /// The screen rectangle the viewport had last frame.
    pub fn rect(&self) -> Rect {
        self.last_rect
    }

    /// Where world point `p` appears on screen (last frame's rectangle and the current camera).
    pub fn screen_pos(&self, p: DVec3) -> Option<Pos2> {
        let r = self.last_rect;
        let view = self
            .camera
            .view((f64::from(r.width()), f64::from(r.height())));
        view.project(p)
            .map(|(x, y)| r.min + Vec2::new(x as f32, y as f32))
    }

    /// True while a gizmo drag is held.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Frames the selection (or everything when it is empty).
    pub fn frame_selection(&mut self, state: &EditorState) {
        match state.selection_bounds() {
            Some(b) => self.camera.frame(b),
            None => self.frame_all(state),
        }
    }

    /// Frames every part in Workspace.
    pub fn frame_all(&mut self, state: &EditorState) {
        let parts = state
            .world
            .root("Workspace")
            .map(|ws| {
                let mut out = Vec::new();
                let mut seen = std::collections::HashSet::new();
                crate::selection::collect_parts(&state.world, ws, &mut out, &mut seen);
                out
            })
            .unwrap_or_default();
        if let Some(b) = Aabb::of_parts(&state.world, &parts) {
            self.camera.frame(b);
        }
    }

    /// Handles a UI request aimed at the viewport.
    pub fn handle(&mut self, request: UiRequest, state: &EditorState) {
        match request {
            UiRequest::FrameSelection => self.frame_selection(state),
            UiRequest::FrameAll => self.frame_all(state),
            _ => {}
        }
    }

    /// Draws the viewport into `ui` and handles its input.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut EditorState,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        if state.play.view().is_some() {
            self.play_ui(ui, state, render_state);
            return;
        }
        if self.showing_play {
            self.leave_play();
        }
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.last_rect = rect;
        self.picks.sync(&state.world);
        let size = (f64::from(rect.width()), f64::from(rect.height()));

        self.navigate(ui, &response, rect, state);
        let view = self.camera.view(size);
        let ppp = ui.ctx().pixels_per_point();
        let px = (
            ((rect.width() * ppp).round() as u32).max(1),
            ((rect.height() * ppp).round() as u32).max(1),
        );

        self.render(state, render_state, px);
        let painter = ui.painter_at(rect);
        match self.gpu.as_ref().and_then(|g| g.texture_id) {
            Some(id) => {
                painter.image(
                    id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.rect_filled(rect, 0.0, ui_tokens().deep);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "The 3D view needs a GPU (wgpu)",
                    ui_tokens().body(),
                    ui_tokens().text_dim,
                );
            }
        }

        let pointer = response.hover_pos();
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let frame = self.gizmo_frame(state, &view);

        // Hover: gizmo handles first, then parts.
        if self.drag.is_none() {
            self.hovered_handle = match (frame, pointer) {
                (Some(f), Some(p)) => f.hit(&view, local(p)),
                _ => None,
            };
        }
        // One pick per pointer position, world change or camera move (not one per frame).
        let pick = pointer.map(|p| {
            let key = (p, state.world.change_cursor(), self.camera, rect);
            match &self.hover_pick {
                Some((k, hit)) if *k == key => *hit,
                _ => {
                    let (x, y) = local(p);
                    let hit = self.picks.first(&view.ray(x, y), &[]);
                    self.hover_pick = Some((key, hit));
                    hit
                }
            }
        });
        if self.drag.is_none() {
            self.hovered_part = match pick {
                Some(hit) if self.hovered_handle.is_none() => hit.map(|h| h.id),
                _ => None,
            };
        }
        if let (Some(p), Some(hit)) = (pointer, pick) {
            let (x, y) = local(p);
            let ray = view.ray(x, y);
            let at = hit.map_or(ray.at(20.0), |h| ray.at(h.distance) + DVec3::Y * 0.5);
            state.insert_point = from_d(at);
        }

        self.pointer_buttons(ui, &response, rect, &view, frame, state);

        // Outlines, then the gizmo on top.
        self.paint_outlines(&painter, rect.min, &view, state);
        if let Some(f) = frame {
            let shown = match &self.drag {
                Some(d) if matches!(d.handle, Handle::Ring(_)) => d.frame(),
                _ => f,
            };
            gizmo::paint(
                &painter,
                rect.min,
                &view,
                &shown,
                self.hovered_handle,
                self.drag.as_ref(),
                pointer,
            );
        }
        if let Press::Box { from } = self.press
            && let Some(p) = pointer
        {
            let r = Rect::from_two_pos(from, p);
            painter.rect_filled(r, 0.0, ui_tokens().accent.gamma_multiply(0.12));
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, ui_tokens().accent),
                egui::StrokeKind::Inside,
            );
        }
        // Camera speed note while flying.
        if response.ctx.input(|i| i.pointer.secondary_down()) && response.hovered() {
            painter.text(
                rect.left_bottom() + Vec2::new(12.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                format!("Camera speed {:.0} studs/s", self.camera.speed),
                ui_tokens().small(),
                ui_tokens().text_dim,
            );
        }
    }

    /// Draws the running game: every frame, from the game camera, GUI on top.
    fn play_ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.last_rect = rect;
        self.play_hovered = response.hovered() || response.dragged();
        self.showing_play = true;
        self.rendered = None;
        self.hovered_part = None;
        self.hovered_handle = None;
        let ppp = ui.ctx().pixels_per_point();
        let px = (
            ((rect.width() * ppp).round() as u32).max(1),
            ((rect.height() * ppp).round() as u32).max(1),
        );
        if self.ensure_gpu(state, render_state)
            && let Some(gpu) = &mut self.gpu
        {
            gpu.ensure_target(px);
            if let (Some(view), Some((_, target, _))) = (state.play.view(), &gpu.target) {
                view.render(&mut gpu.renderer, target, px);
            }
        }
        let painter = ui.painter_at(rect);
        match self.gpu.as_ref().and_then(|g| g.texture_id) {
            Some(id) => {
                painter.image(
                    id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.rect_filled(rect, 0.0, ui_tokens().deep);
            }
        }
        if let Some(view) = state.play.view() {
            view.paint_gui(&painter);
        }
        painter.rect_stroke(
            rect.shrink(1.0),
            0.0,
            Stroke::new(2.0, theme::success()),
            egui::StrokeKind::Inside,
        );
        ui.ctx().request_repaint();
    }

    /// Back from Play: the renderer still holds the play world and the avatar.
    fn leave_play(&mut self) {
        if let Some(gpu) = &mut self.gpu {
            gpu.renderer.set_skinned(&[]);
        }
        self.rendered = None;
        self.hover_pick = None;
        self.showing_play = false;
        self.play_hovered = false;
    }

    /// Creates the GPU side on first use; false (logged once) without a GPU.
    fn ensure_gpu(&mut self, state: &EditorState, rs: Option<&egui_wgpu::RenderState>) -> bool {
        if self.gpu.is_none() {
            match rs {
                Some(rs) => self.gpu = Some(Gpu::new(rs)),
                None => {
                    if !self.gpu_missing_logged {
                        state.output.studio("No GPU renderer: the 3D view is off");
                        self.gpu_missing_logged = true;
                    }
                    return false;
                }
            }
        }
        true
    }

    fn render(&mut self, state: &EditorState, rs: Option<&egui_wgpu::RenderState>, px: (u32, u32)) {
        if !self.ensure_gpu(state, rs) {
            return;
        }
        let Some(gpu) = &mut self.gpu else { return };
        let key = (state.world.change_cursor(), self.camera, px);
        if self.rendered == Some(key) {
            return;
        }
        gpu.ensure_target(px);
        gpu.renderer.sync(&state.world);
        if let Some((_, view, _)) = &gpu.target {
            gpu.renderer.render(view, &self.camera.to_render(), px);
        }
        self.rendered = Some(key);
    }

    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        state: &EditorState,
    ) {
        let ctx = ui.ctx().clone();
        let (delta, secondary, middle, alt, primary, scroll, dt) = ctx.input(|i| {
            (
                i.pointer.delta(),
                i.pointer.secondary_down(),
                i.pointer.middle_down(),
                i.modifiers.alt,
                i.pointer.primary_down(),
                i.smooth_scroll_delta.y,
                f64::from(i.stable_dt.min(0.1)),
            )
        });
        let over = response.hovered() || response.dragged() || response.is_pointer_button_down_on();
        if !over {
            return;
        }
        let (dx, dy) = (f64::from(delta.x), f64::from(delta.y));
        if secondary {
            self.camera.rotate(dx, dy);
            let key = |k| ctx.input(|i| i.key_down(k));
            let mut mv = DVec3::ZERO;
            if key(egui::Key::W) {
                mv.z += 1.0;
            }
            if key(egui::Key::S) {
                mv.z -= 1.0;
            }
            if key(egui::Key::D) {
                mv.x += 1.0;
            }
            if key(egui::Key::A) {
                mv.x -= 1.0;
            }
            if key(egui::Key::E) {
                mv.y += 1.0;
            }
            if key(egui::Key::Q) {
                mv.y -= 1.0;
            }
            let fast = ctx.input(|i| i.modifiers.shift);
            if mv != DVec3::ZERO {
                self.camera.fly(mv, dt, fast);
            }
            if scroll != 0.0 {
                self.camera.change_speed(f64::from(scroll) / 50.0);
            }
            ctx.request_repaint();
        } else if middle {
            self.camera.pan(dx, dy, f64::from(rect.height()));
        } else if alt && primary && self.drag.is_none() {
            self.camera.orbit(dx, dy);
        } else if scroll != 0.0 && response.hovered() {
            let toward = match response.hover_pos() {
                Some(p) => {
                    let view = self
                        .camera
                        .view((f64::from(rect.width()), f64::from(rect.height())));
                    view.ray(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y))
                        .dir
                }
                None => self.camera.forward(),
            };
            self.camera.dolly(toward, f64::from(scroll) / 50.0);
        }
        let _ = state;
    }

    fn gizmo_frame(&self, state: &EditorState, view: &ViewProjection) -> Option<GizmoFrame> {
        if state.tool == Tool::Select || state.selection.is_empty() || state.play.is_playing() {
            return None;
        }
        let parts = state.selection.parts(&state.world);
        let active_part = state
            .selection
            .active()
            .filter(|&a| state.world.is_a(a, "BasePart"))
            .and_then(|a| PartTransform::read(&state.world, a));
        let origin = if state.selection.len() == 1 && active_part.is_some() {
            active_part.map(|t| t.position)?
        } else {
            Aabb::of_parts(&state.world, &parts)?.centre()
        };
        let rotation = match (state.tool, state.space, active_part) {
            (Tool::Scale, _, Some(t)) | (_, Space::Local, Some(t)) => t.rotation,
            _ => DQuat::IDENTITY,
        };
        Some(GizmoFrame::new(origin, rotation, state.tool, view))
    }

    fn pointer_buttons(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        view: &ViewProjection,
        frame: Option<GizmoFrame>,
        state: &mut EditorState,
    ) {
        let ctx = ui.ctx().clone();
        let (pressed, released, secondary_pressed, mods, pos) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.secondary_pressed(),
                i.modifiers,
                i.pointer.interact_pos(),
            )
        });
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));

        // Gizmo drag in progress.
        if let Some(drag) = &mut self.drag {
            let cancel = secondary_pressed
                || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if cancel {
                drag.cancel(&mut state.world);
                self.drag = None;
                state.note("Drag cancelled".into());
                return;
            }
            if let Some(p) = pos {
                let (x, y) = local(p);
                let m = DragModifiers {
                    invert_snap: mods.command,
                    precision: mods.shift,
                };
                drag.update(&view.ray(x, y), &state.snapping, m);
                drag.apply(&mut state.world);
            }
            if released || !ctx.input(|i| i.pointer.primary_down()) {
                if let Some(cmd) = drag.finish(&state.world) {
                    state.record(cmd);
                }
                self.drag = None;
            }
            ctx.request_repaint();
            return;
        }

        if pressed && response.hovered() && !mods.alt {
            let Some(p) = pos else { return };
            if let (Some(f), Some(h)) = (frame, self.hovered_handle) {
                let (x, y) = local(p);
                let parts = state.selection.parts(&state.world);
                self.drag = GizmoDrag::begin(&state.world, f, h, &view.ray(x, y), &parts);
                self.press = Press::None;
                return;
            }
            self.press = Press::Pending { at: p };
        }
        if pressed
            && response.hovered()
            && mods.alt
            && let Some(p) = pos
        {
            self.press = Press::Alt { at: p };
        }
        match self.press {
            Press::Pending { at } => {
                if let Some(p) = pos
                    && (p - at).length() > DRAG_SLOP
                    && ctx.input(|i| i.pointer.primary_down())
                {
                    self.press = Press::Box { from: at };
                } else if released {
                    self.click(state, mods.command);
                    self.press = Press::None;
                }
            }
            Press::Box { from } => {
                if released {
                    if let Some(p) = pos {
                        let r = Rect::from_two_pos(from, p);
                        let inside = self.picks.filter_centres(|c| {
                            view.project(c).is_some_and(|(x, y)| {
                                r.contains(rect.min + Vec2::new(x as f32, y as f32))
                            })
                        });
                        if mods.command {
                            for id in inside {
                                if !state.selection.contains(id) {
                                    state.selection.add(id);
                                }
                            }
                        } else {
                            state.selection.set(inside);
                        }
                    }
                    self.press = Press::None;
                }
            }
            Press::Alt { at } => {
                if released {
                    // A click without travel cycles through everything under the cursor.
                    if let Some(p) = pos
                        && (p - at).length() <= DRAG_SLOP
                    {
                        let (x, y) = local(p);
                        self.picks.all(&view.ray(x, y), &mut self.hits);
                        if !self.hits.is_empty() {
                            self.cycle = (self.cycle + 1) % self.hits.len();
                            state.selection.select_only(self.hits[self.cycle].id);
                        }
                    }
                    self.press = Press::None;
                }
            }
            Press::None => {}
        }
    }

    /// Click selection with Model-first rule.
    fn click(&mut self, state: &mut EditorState, ctrl: bool) {
        let Some(part) = self.hovered_part else {
            if !ctrl {
                state.selection.clear();
            }
            self.last_click = None;
            return;
        };
        let model = outermost_model(state, part);
        let target = match model {
            Some(m) if !(state.selection.ids() == [m] && self.last_click == Some(part)) => m,
            _ => part,
        };
        if ctrl {
            state.selection.toggle(target);
        } else {
            state.selection.select_only(target);
        }
        self.last_click = Some(part);
        self.cycle = 0;
    }

    fn paint_outlines(
        &self,
        painter: &egui::Painter,
        offset: Pos2,
        view: &ViewProjection,
        state: &EditorState,
    ) {
        let to = |p: DVec3| {
            view.project(p)
                .map(|(x, y)| offset + Vec2::new(x as f32, y as f32))
        };
        let mut drawn = 0;
        for &id in state.selection.ids() {
            if drawn >= MAX_OUTLINES {
                break;
            }
            if state.world.is_a(id, "BasePart") {
                if let Some(t) = self.picks.transform(id) {
                    box_edges(
                        painter,
                        &to,
                        &t.corners(),
                        Stroke::new(2.0, ui_tokens().accent),
                    );
                    drawn += 1;
                }
            } else if let Some(b) = Aabb::of_parts(
                &state.world,
                &crate::selection::Selection::parts_of(&state.world, id),
            ) {
                // A Model or folder draws its bounding-box corners, so it reads differently from a part.
                box_corners(painter, &to, b, Stroke::new(2.0, ui_tokens().accent));
                drawn += 1;
            }
        }
        if let Some(h) = self.hovered_part
            && !state.selection.contains(h)
            && let Some(t) = self.picks.transform(h)
        {
            box_edges(
                painter,
                &to,
                &t.corners(),
                Stroke::new(1.0, ui_tokens().accent.gamma_multiply(0.6)),
            );
        }
    }
}

/// The outermost Model between `part` and Workspace.
fn outermost_model(state: &EditorState, part: InstanceId) -> Option<InstanceId> {
    let ws = state.world.root("Workspace")?;
    let mut found = None;
    let mut cur = state.world.parent(part);
    while let Some(c) = cur {
        if c == ws {
            break;
        }
        if state.world.is_a(c, "Model") {
            found = Some(c);
        }
        cur = state.world.parent(c);
    }
    found
}

const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

fn box_edges(
    painter: &egui::Painter,
    to: &dyn Fn(DVec3) -> Option<Pos2>,
    corners: &[DVec3; 8],
    stroke: Stroke,
) {
    let mut pts = [None; 8];
    for (p, c) in pts.iter_mut().zip(corners) {
        *p = to(*c);
    }
    for (a, b) in EDGES {
        if let (Some(a), Some(b)) = (pts[a], pts[b]) {
            painter.line_segment([a, b], stroke);
        }
    }
}

fn box_corners(
    painter: &egui::Painter,
    to: &dyn Fn(DVec3) -> Option<Pos2>,
    b: Aabb,
    stroke: Stroke,
) {
    let size = b.size();
    let corner = |i: usize| {
        DVec3::new(
            if i & 1 == 0 { b.min.x } else { b.max.x },
            if i & 2 == 0 { b.min.y } else { b.max.y },
            if i & 4 == 0 { b.min.z } else { b.max.z },
        )
    };
    for i in 0..8 {
        let c = corner(i);
        let Some(sc) = to(c) else { continue };
        for (axis, len) in [(0usize, size.x), (1, size.y), (2, size.z)] {
            let sign = if i & (1 << axis) == 0 { 1.0 } else { -1.0 };
            let mut d = DVec3::ZERO;
            d[axis] = sign * (len * 0.2).min(2.0);
            if let Some(e) = to(c + d) {
                painter.line_segment([sc, e], stroke);
            }
        }
    }
}
