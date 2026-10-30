//! The editor: state, undo history and the panel layout.
//!
//! Layout follows the Studio design doc (5.11): a title bar, the library and effect outline on the left,
//! the viewport (or node canvas, or source) in the centre with its own header, the timeline at the bottom,
//! the module stack inspector on the right and a status bar.

use crate::inspector::{self, Stage};
use crate::preview::Preview;
use crate::theme::{self, color};
use crate::widgets::node_canvas::{self, CanvasView, Group, Node, NodeRow, Side, Wire};
use crate::widgets::timeline::{self, Track};
use crate::widgets::{self, Icon};
use egui::{CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use metallic_vfx::draw::build_draw_list;
use metallic_vfx::{
    ArtStyle, Beam, Effect, Emitter, SpawnModule, Simulation, Trail, VfxError, presets,
};
use std::collections::HashMap;

/// What the inspector shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Effect settings.
    Effect,
    /// An emitter's module stack.
    Emitter(usize),
    /// A trail.
    Trail(usize),
    /// A beam.
    Beam(usize),
}

/// The centre view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Live preview (the default).
    Stack,
    /// Node graph of the emitters, with a preview inset.
    Graph,
    /// The `.vfx.ron` text.
    Source,
}

/// Preview step used when scrubbing (seconds).
pub const SCRUB_STEP: f32 = 1.0 / 60.0;
const UNDO_LIMIT: usize = 200;

/// The VFX editor. Embed it with [`VfxEditor::ui`]; it needs only an `egui::Ui`.
pub struct VfxEditor {
    effect: Effect,
    committed: Effect,
    undo: Vec<Effect>,
    redo: Vec<Effect>,
    dirty: bool,
    sim: Simulation,
    /// Playing (advances with frame time).
    pub playing: bool,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Preview restyle (`None` shows the effect as authored).
    pub style_view: Option<ArtStyle>,
    /// Centre view.
    pub view: View,
    /// Inspector selection.
    pub selection: Selection,
    focus: Option<(Stage, usize)>,
    preview: Preview,
    canvas: CanvasView,
    node_pos: HashMap<u64, Pos2>,
    filter: String,
    source: String,
    source_error: Option<String>,
    status: String,
    seed: u32,
    library: Vec<Effect>,
    preset: Option<usize>,
}

impl VfxEditor {
    /// Opens the editor on the first preset.
    ///
    /// # Errors
    /// Fails only if a built-in preset does not validate.
    pub fn new() -> Result<Self, VfxError> {
        let library = presets::all();
        let effect = library.first().cloned().unwrap_or_default();
        let sim = Simulation::new(effect.clone(), 1)?;
        let mut editor = Self {
            committed: effect.clone(),
            effect,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            sim,
            playing: true,
            speed: 1.0,
            style_view: None,
            view: View::Stack,
            selection: Selection::Emitter(0),
            focus: None,
            preview: Preview::default(),
            canvas: CanvasView::default(),
            node_pos: HashMap::new(),
            filter: String::new(),
            source: String::new(),
            source_error: None,
            status: "Ready".into(),
            seed: 1,
            library,
            preset: Some(0),
        };
        editor.frame_camera();
        Ok(editor)
    }

    /// The effect being edited.
    #[must_use]
    pub fn effect(&self) -> &Effect {
        &self.effect
    }

    /// The live simulation.
    #[must_use]
    pub fn simulation(&self) -> &Simulation {
        &self.sim
    }

    /// Opens an effect (clears undo history).
    ///
    /// # Errors
    /// [`VfxError`] when the effect does not validate.
    pub fn open(&mut self, effect: Effect) -> Result<(), VfxError> {
        self.sim = Simulation::new(self.shown(&effect), self.seed)?;
        self.committed = effect.clone();
        self.effect = effect;
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.selection = if self.effect.emitters.is_empty() {
            if self.effect.beams.is_empty() {
                Selection::Trail(0)
            } else {
                Selection::Beam(0)
            }
        } else {
            Selection::Emitter(0)
        };
        self.focus = None;
        self.node_pos.clear();
        self.canvas = CanvasView::default();
        self.source_error = None;
        self.status = format!("Opened {}", self.effect.name);
        self.frame_camera();
        Ok(())
    }

    /// Opens a preset by name.
    ///
    /// # Errors
    /// [`VfxError::NoSuchPreset`] for unknown names.
    pub fn open_preset(&mut self, name: &str) -> Result<(), VfxError> {
        let effect = presets::by_name(name)?;
        self.preset = self.library.iter().position(|e| e.name == effect.name);
        self.open(effect)
    }

    /// Jumps the preview to `t` seconds (stepping at [`SCRUB_STEP`], following the preview motion).
    pub fn seek(&mut self, t: f32) {
        let motion = self.effect.preview;
        self.sim.restart();
        self.sim.teleport(motion.transform_at(0.0));
        let steps = (t.max(0.0) / SCRUB_STEP).round() as u32;
        for i in 0..steps.min(60 * 120) {
            self.sim
                .set_transform(motion.transform_at((i + 1) as f32 * SCRUB_STEP));
            self.sim.step(SCRUB_STEP);
        }
    }

    /// Shows the preview restyled to `style` (`None`: as authored). Restarts the simulation.
    pub fn set_style_view(&mut self, style: Option<ArtStyle>) {
        self.style_view = style;
        match Simulation::new(self.shown(&self.effect), self.seed) {
            Ok(s) => self.sim = s,
            Err(e) => self.status = format!("Invalid: {e}"),
        }
        self.frame_camera();
    }

    /// Draws only the live preview into `rect` (contact sheets, thumbnails, embedding).
    pub fn preview_ui(&mut self, ui: &mut Ui, rect: Rect) {
        self.viewport(ui, rect, false);
    }

    /// Sets the inspector selection and focused module (used by the graph view and screenshots).
    pub fn select(&mut self, selection: Selection, focus: Option<(Stage, usize)>) {
        self.selection = selection;
        self.focus = focus;
    }

    /// Advances the preview by `dt` seconds of wall time.
    pub fn tick(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        let dt = (dt * self.speed).clamp(0.0, 0.1);
        let t = self.sim.time() + dt;
        self.sim.set_transform(self.effect.preview.transform_at(t));
        self.sim.step(dt);
        if self.sim.is_finished() {
            self.sim.restart();
        }
    }

    fn shown(&self, effect: &Effect) -> Effect {
        match self.style_view {
            Some(s) if s != effect.style => effect.restyle(s),
            _ => effect.clone(),
        }
    }

    fn shown_style(&self) -> ArtStyle {
        self.style_view.unwrap_or(self.effect.style)
    }

    /// Pushes the edited effect into the simulation (keeps live particles).
    fn sync(&mut self) {
        let shown = self.shown(&self.effect);
        match self.sim.set_effect(shown) {
            Ok(()) => {
                if self.status.starts_with("Invalid") {
                    self.status = "Ready".into();
                }
            }
            Err(e) => self.status = format!("Invalid: {e}"),
        }
    }

    fn edited(&mut self) {
        self.dirty = true;
        self.sync();
    }

    fn commit_if_idle(&mut self, ui: &Ui) {
        let busy = ui.input(|i| i.pointer.any_down());
        if self.dirty && !busy {
            let before = std::mem::replace(&mut self.committed, self.effect.clone());
            if before != self.committed {
                self.undo.push(before);
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.dirty = false;
        }
    }

    /// Undoes the last committed edit.
    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.effect, prev));
            self.committed = self.effect.clone();
            self.clamp_selection();
            self.sync();
            self.status = "Undo".into();
        }
    }

    /// Redoes the last undone edit.
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.effect, next));
            self.committed = self.effect.clone();
            self.clamp_selection();
            self.sync();
            self.status = "Redo".into();
        }
    }

    fn clamp_selection(&mut self) {
        let ok = match self.selection {
            Selection::Effect => true,
            Selection::Emitter(i) => i < self.effect.emitters.len(),
            Selection::Trail(i) => i < self.effect.trails.len(),
            Selection::Beam(i) => i < self.effect.beams.len(),
        };
        if !ok {
            self.selection = Selection::Effect;
            self.focus = None;
        }
    }

    /// Fits the orbit camera to the effect by simulating a copy for a moment.
    fn frame_camera(&mut self) {
        let Ok(mut probe) = Simulation::new(self.shown(&self.effect), self.seed) else {
            return;
        };
        let mut bounds = None;
        for i in 0..120 {
            let t = (i + 1) as f32 / 60.0;
            probe.set_transform(self.effect.preview.transform_at(t));
            probe.step(1.0 / 60.0);
            if let Some(b) = probe.bounds() {
                bounds = Some(match bounds {
                    None => b,
                    Some(o) => b.union(o),
                });
            }
        }
        let grid = self.preview.grid;
        self.preview = Preview {
            textures: std::mem::take(&mut self.preview.textures),
            grid,
            ..Preview::default()
        };
        if let Some(b) = bounds {
            let c = (b.min + b.max) * 0.5;
            let r = ((b.max - b.min).length() * 0.5).clamp(1.0, 30.0);
            self.preview.target = metallic_vfx::Vec3::new(c.x * 0.5, c.y.max(0.6), c.z * 0.5);
            self.preview.distance = (r * 2.3).clamp(5.0, 45.0);
        }
    }

    fn save(&mut self) {
        let name = presets::file_name(&self.effect.name);
        match self.effect.to_ron() {
            Ok(text) => match std::fs::write(&name, text) {
                Ok(()) => self.status = format!("Saved {name}"),
                Err(e) => self.status = format!("Could not save {name}: {e}"),
            },
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn load_dropped(&mut self, ui: &Ui) {
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let text = f.bytes().ok().and_then(|b| String::from_utf8(b).ok());
            let Some(text) = text else { continue };
            match Effect::from_ron(&text).and_then(|e| self.open(e)) {
                Ok(()) => self.preset = None,
                Err(e) => self.status = format!("Could not open {}: {e}", f.path().display()),
            }
        }
    }

    /// Draws the whole editor into `ui` (normally the root `Ui` of an eframe app).
    pub fn ui(&mut self, ui: &mut Ui) {
        let dt = ui.input(|i| i.stable_dt);
        self.tick(dt);
        if self.playing {
            ui.ctx().request_repaint();
        }
        self.load_dropped(ui);
        let (undo, redo, save, space) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                false,
            )
        });
        let _ = space;
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save {
            self.save();
        }

        let bar = |fill| Frame::new().fill(fill).inner_margin(Margin::symmetric(10, 0));
        egui::Panel::top("vfx-title")
            .exact_size(40.0)
            .frame(bar(color::BG_DEEP))
            .show(ui, |ui| self.title_bar(ui));
        egui::Panel::bottom("vfx-status")
            .exact_size(24.0)
            .frame(bar(color::BG_DEEP))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("vfx-left")
            .exact_size(232.0)
            .resizable(false)
            .frame(theme::panel_frame().stroke(Stroke::NONE))
            .show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("vfx-right")
            .exact_size(372.0)
            .resizable(false)
            .frame(theme::panel_frame())
            .show(ui, |ui| self.right_panel(ui));
        egui::Panel::bottom("vfx-timeline")
            .exact_size(150.0)
            .frame(theme::panel_frame())
            .show(ui, |ui| self.timeline_panel(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(color::BG_BASE))
            .show(ui, |ui| self.centre(ui));
        self.commit_if_idle(ui);
    }

    fn title_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("VFX Editor").font(theme::semibold(14.0)).color(color::TEXT));
            ui.add_space(12.0);
            ui.menu_button("File", |ui| {
                if ui.button("Save as .vfx.ron    Ctrl+S").clicked() {
                    self.save();
                    ui.close();
                }
                if ui.button("Copy as text").clicked() {
                    if let Ok(t) = self.effect.to_ron() {
                        ui.ctx().copy_text(t);
                        self.status = "Copied effect text".into();
                    }
                    ui.close();
                }
                ui.label(RichText::new("Drop a .vfx.ron file to open it").color(color::TEXT_3));
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo    Ctrl+Z")).clicked() {
                    self.undo();
                    ui.close();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo    Ctrl+Y")).clicked() {
                    self.redo();
                    ui.close();
                }
            });
            ui.add_space(16.0);
            ui.label(RichText::new(&self.effect.name).color(color::TEXT_2));
            if !self.undo.is_empty() || self.dirty {
                ui.label(RichText::new("edited").color(color::TEXT_3));
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            let c = if self.status.starts_with("Invalid") || self.status.starts_with("Could not") {
                color::ERROR
            } else {
                color::TEXT_3
            };
            ui.label(RichText::new(&self.status).color(c).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "CPU reference sim · seed {} · {} particles",
                        self.seed,
                        self.sim.particle_count()
                    ))
                    .color(color::TEXT_3)
                    .size(11.0),
                );
            });
        });
    }

    fn section_title(ui: &mut Ui, text: &str) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(text).font(theme::semibold(14.0)).color(color::TEXT));
        });
        ui.add_space(4.0);
    }

    fn list_row(ui: &mut Ui, selected: bool, label: &str, detail: &str, dim: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
        let r = rect.shrink2(Vec2::new(6.0, 0.0));
        let fill = if selected {
            color::SELECT
        } else if resp.hovered() {
            color::BG_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, theme::R_SM, fill);
        let text = if dim { color::TEXT_3 } else { color::TEXT };
        ui.painter().text(
            Pos2::new(r.min.x + 10.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            text,
        );
        ui.painter().text(
            Pos2::new(r.max.x - 8.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            detail,
            egui::FontId::proportional(11.0),
            color::TEXT_3,
        );
        resp
    }

    fn left_panel(&mut self, ui: &mut Ui) {
        Self::section_title(ui, "Effect");
        let mut changed = false;
        let mut new_selection = None;
        if Self::list_row(ui, self.selection == Selection::Effect, &self.effect.name, self.effect.style.label(), false)
            .clicked()
        {
            new_selection = Some(Selection::Effect);
        }
        for (i, em) in self.effect.emitters.iter().enumerate() {
            let n = em.init.len() + em.update.len() + em.spawn.len();
            if Self::list_row(
                ui,
                self.selection == Selection::Emitter(i),
                &format!("   {}", em.name),
                &format!("{n} modules"),
                !em.enabled,
            )
            .clicked()
            {
                new_selection = Some(Selection::Emitter(i));
            }
        }
        for (i, t) in self.effect.trails.iter().enumerate() {
            if Self::list_row(ui, self.selection == Selection::Trail(i), &format!("   {}", t.name), "Trail", !t.enabled)
                .clicked()
            {
                new_selection = Some(Selection::Trail(i));
            }
        }
        for (i, b) in self.effect.beams.iter().enumerate() {
            if Self::list_row(ui, self.selection == Selection::Beam(i), &format!("   {}", b.name), "Beam", !b.enabled)
                .clicked()
            {
                new_selection = Some(Selection::Beam(i));
            }
        }
        if let Some(s) = new_selection {
            self.select(s, None);
        }
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.menu_button(RichText::new("+ Add").color(color::ICON), |ui| {
                if ui.button("Emitter").clicked() {
                    self.effect.emitters.push(Emitter {
                        name: format!("Emitter {}", self.effect.emitters.len() + 1),
                        ..Emitter::default()
                    });
                    self.selection = Selection::Emitter(self.effect.emitters.len() - 1);
                    changed = true;
                    ui.close();
                }
                if ui.button("Trail").clicked() {
                    self.effect.trails.push(Trail::default());
                    self.selection = Selection::Trail(self.effect.trails.len() - 1);
                    changed = true;
                    ui.close();
                }
                if ui.button("Beam").clicked() {
                    self.effect.beams.push(Beam::default());
                    self.selection = Selection::Beam(self.effect.beams.len() - 1);
                    changed = true;
                    ui.close();
                }
            });
            let removable = !matches!(self.selection, Selection::Effect);
            if ui.add_enabled(removable, egui::Button::new("Duplicate")).clicked() {
                match self.selection {
                    Selection::Emitter(i) => {
                        let mut e = self.effect.emitters[i].clone();
                        e.name.push_str(" copy");
                        self.effect.emitters.insert(i + 1, e);
                        self.selection = Selection::Emitter(i + 1);
                    }
                    Selection::Trail(i) => {
                        let t = self.effect.trails[i].clone();
                        self.effect.trails.insert(i + 1, t);
                    }
                    Selection::Beam(i) => {
                        let b = self.effect.beams[i].clone();
                        self.effect.beams.insert(i + 1, b);
                    }
                    Selection::Effect => {}
                }
                changed = true;
            }
            if ui.add_enabled(removable, egui::Button::new("Remove")).clicked() {
                match self.selection {
                    Selection::Emitter(i) => {
                        self.effect.emitters.remove(i);
                    }
                    Selection::Trail(i) => {
                        self.effect.trails.remove(i);
                    }
                    Selection::Beam(i) => {
                        self.effect.beams.remove(i);
                    }
                    Selection::Effect => {}
                }
                self.selection = Selection::Effect;
                changed = true;
            }
        });
        if changed {
            // Structural edits change the number of emitters: rebuild the simulation.
            self.dirty = true;
            match Simulation::new(self.shown(&self.effect), self.seed) {
                Ok(s) => self.sim = s,
                Err(e) => self.status = format!("Invalid: {e}"),
            }
        }

        ui.add_space(6.0);
        let sep = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [Pos2::new(sep.min.x, sep.min.y), Pos2::new(sep.max.x, sep.min.y)],
            Stroke::new(1.0, color::LINE),
        );
        Self::section_title(ui, "Library");
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter presets")
                    .desired_width(ui.available_width() - 10.0),
            );
        });
        ui.add_space(4.0);
        let filter = self.filter.to_lowercase();
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("library").show(ui, |ui| {
            for (i, e) in self.library.iter().enumerate() {
                let hay = format!("{} {}", e.name, e.tags.join(" ")).to_lowercase();
                if !filter.is_empty() && !hay.contains(&filter) {
                    continue;
                }
                let detail = if e.style == ArtStyle::Realistic {
                    e.tags.first().cloned().unwrap_or_default()
                } else {
                    e.style.label().to_string()
                };
                if Self::list_row(ui, self.preset == Some(i), &e.name, &detail, false)
                    .on_hover_text(&e.description)
                    .clicked()
                {
                    open = Some(i);
                }
            }
        });
        if let Some(i) = open
            && let Some(e) = self.library.get(i).cloned()
        {
            match self.open(e) {
                Ok(()) => self.preset = Some(i),
                Err(err) => self.status = format!("Could not open preset: {err}"),
            }
        }
    }

    fn right_panel(&mut self, ui: &mut Ui) {
        let title = match self.selection {
            Selection::Effect => "Effect".to_string(),
            Selection::Emitter(i) => self
                .effect
                .emitters
                .get(i)
                .map_or_else(String::new, |e| format!("Emitter · {}", e.name)),
            Selection::Trail(_) => "Trail".to_string(),
            Selection::Beam(_) => "Beam".to_string(),
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(title).font(theme::semibold(14.0)).color(color::TEXT));
        });
        ui.add_space(6.0);
        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt("inspector")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 10, right: 12, top: 0, bottom: 12 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match self.selection {
                        Selection::Effect => {
                            changed |= inspector::effect_settings(ui, &mut self.effect);
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!(
                                    "{} emitters · {} trails · {} beams",
                                    self.effect.emitters.len(),
                                    self.effect.trails.len(),
                                    self.effect.beams.len()
                                ))
                                .color(color::TEXT_3),
                            );
                        }
                        Selection::Emitter(i) => {
                            if let Some(em) = self.effect.emitters.get_mut(i) {
                                changed |= inspector::emitter_stack(ui, em, i, self.focus, &mut self.preview);
                            }
                        }
                        Selection::Trail(i) => {
                            if let Some(t) = self.effect.trails.get_mut(i) {
                                changed |= inspector::trail_inspector(ui, t, i, &mut self.preview);
                            }
                        }
                        Selection::Beam(i) => {
                            if let Some(b) = self.effect.beams.get_mut(i) {
                                changed |= inspector::beam_inspector(ui, b, i, &mut self.preview);
                            }
                        }
                    }
                });
            });
        if changed {
            self.edited();
        }
    }

    fn tracks(&self) -> Vec<Track> {
        let len = self.timeline_length();
        let mut tracks = Vec::new();
        for em in &self.effect.emitters {
            let mut markers = Vec::new();
            let mut spans = Vec::new();
            for s in &em.spawn {
                if s.is_disabled() {
                    continue;
                }
                match s {
                    SpawnModule::Burst {
                        time,
                        cycles,
                        interval,
                        ..
                    } => {
                        for k in 0..(*cycles).min(64) {
                            let t = em.delay + time + k as f32 * interval;
                            if t <= len {
                                markers.push(t);
                            }
                        }
                    }
                    SpawnModule::Rate { .. } | SpawnModule::Distance { .. } => {
                        let end = if self.effect.duration > 0.0 && !self.effect.looping {
                            self.effect.duration + em.delay
                        } else {
                            len
                        };
                        spans.push((em.delay, end.min(len)));
                    }
                }
            }
            tracks.push(Track {
                label: em.name.clone(),
                markers,
                spans,
                enabled: em.enabled,
            });
        }
        for t in &self.effect.trails {
            tracks.push(Track {
                label: t.name.clone(),
                markers: Vec::new(),
                spans: vec![(0.0, len)],
                enabled: t.enabled,
            });
        }
        for b in &self.effect.beams {
            tracks.push(Track {
                label: b.name.clone(),
                markers: Vec::new(),
                spans: vec![(0.0, len)],
                enabled: b.enabled,
            });
        }
        tracks
    }

    fn timeline_length(&self) -> f32 {
        if self.effect.duration > 0.0 {
            self.effect.duration
        } else {
            4.0
        }
    }

    fn timeline_panel(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let header = Rect::from_min_size(full.min, Vec2::new(full.width(), 36.0));
        ui.painter().line_segment(
            [full.left_top(), full.right_top()],
            Stroke::new(1.0, color::LINE),
        );
        let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header.shrink2(Vec2::new(10.0, 5.0))));
        header_ui.horizontal_centered(|ui| {
            if widgets::icon_button(ui, Icon::Restart, false, "Restart").clicked() {
                self.sim.restart();
                self.sim.teleport(self.effect.preview.transform_at(0.0));
            }
            let icon = if self.playing { Icon::Pause } else { Icon::Play };
            if widgets::icon_button(ui, icon, false, "Play / pause").clicked() {
                self.playing = !self.playing;
            }
            if widgets::icon_button(ui, Icon::StepForward, false, "Step one frame").clicked() {
                self.playing = false;
                let t = self.sim.time() + SCRUB_STEP;
                self.sim.set_transform(self.effect.preview.transform_at(t));
                self.sim.step(SCRUB_STEP);
            }
            ui.add_space(6.0);
            let len = self.timeline_length();
            let shown = if self.effect.duration > 0.0 { self.sim.time() % len } else { self.sim.time() };
            ui.label(
                RichText::new(format!("{shown:.2} / {len:.2} s"))
                    .font(egui::FontId::monospace(12.0))
                    .color(color::TEXT),
            );
            ui.add_space(10.0);
            egui::ComboBox::from_id_salt("speed")
                .width(64.0)
                .selected_text(format!("{}×", self.speed))
                .show_ui(ui, |ui| {
                    for s in [0.1, 0.25, 0.5, 1.0, 2.0] {
                        ui.selectable_value(&mut self.speed, s, format!("{s}×"));
                    }
                });
            ui.add_space(6.0);
            if ui
                .button(format!("Seed {}", self.seed))
                .on_hover_text("New random seed (the simulation is deterministic per seed)")
                .clicked()
            {
                self.seed = self.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) % 10_000;
                self.sim.reseed(self.seed);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut looping = self.effect.looping;
                if ui
                    .checkbox(&mut looping, "Loop")
                    .on_hover_text("Effect repeats every cycle")
                    .changed()
                {
                    self.effect.looping = looping;
                    self.edited();
                }
            });
        });
        let body = Rect::from_min_max(Pos2::new(full.min.x, header.max.y), full.max);
        ui.painter().line_segment([body.left_top(), body.right_top()], Stroke::new(1.0, color::LINE));
        let len = self.timeline_length();
        let shown = if self.effect.duration > 0.0 { self.sim.time() % len } else { self.sim.time().min(len) };
        let tracks = self.tracks();
        if let Some(t) = timeline::timeline(ui, body, shown, len, &tracks) {
            self.playing = false;
            self.seek(t);
        }
    }

    fn centre(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let header = Rect::from_min_size(full.min, Vec2::new(full.width(), 36.0));
        ui.painter().rect_filled(header, 0.0, color::BG_PANEL);
        ui.painter().line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0, color::LINE),
        );
        let mut hui = ui.new_child(egui::UiBuilder::new().max_rect(header.shrink2(Vec2::new(10.0, 5.0))));
        hui.horizontal_centered(|ui| {
            let views = ["Preview", "Graph", "Source"];
            let current = match self.view {
                View::Stack => 0,
                View::Graph => 1,
                View::Source => 2,
            };
            if let Some(i) = widgets::segmented(ui, &views, current) {
                self.view = [View::Stack, View::Graph, View::Source][i];
                if self.view == View::Source {
                    self.source = self.effect.to_ron().unwrap_or_default();
                    self.source_error = None;
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Style").color(color::TEXT_2));
            let styles: Vec<&str> = std::iter::once("Authored")
                .chain(ArtStyle::ALL.iter().map(|s| s.label()))
                .collect();
            let current = match self.style_view {
                None => 0,
                Some(s) => ArtStyle::ALL.iter().position(|x| *x == s).map_or(0, |p| p + 1),
            };
            if let Some(i) = widgets::segmented(ui, &styles, current) {
                let style = if i == 0 { None } else { ArtStyle::ALL.get(i - 1).copied() };
                let t = self.sim.time();
                self.set_style_view(style);
                self.seek(t.min(3.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Grid, self.preview.grid, "Ground grid").clicked() {
                    self.preview.grid = !self.preview.grid;
                }
                egui::ComboBox::from_id_salt("motion")
                    .width(76.0)
                    .selected_text(self.effect.preview.label())
                    .show_ui(ui, |ui| {
                        for m in metallic_vfx::effect::PreviewMotion::ALL {
                            if ui.selectable_value(&mut self.effect.preview, m, m.label()).changed() {
                                self.dirty = true;
                            }
                        }
                    });
                ui.label(RichText::new("Motion").color(color::TEXT_2));
            });
        });

        let body = Rect::from_min_max(Pos2::new(full.min.x, header.max.y + 1.0), full.max);
        match self.view {
            View::Stack => self.viewport(ui, body, true),
            View::Graph => {
                self.graph(ui, body);
                let inset = Rect::from_min_size(
                    Pos2::new(body.max.x - 300.0, body.max.y - 216.0),
                    Vec2::new(284.0, 200.0),
                );
                let painter = ui.painter();
                painter.add(egui::Shape::from(
                    egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(115),
                    }
                    .as_shape(inset, CornerRadius::same(theme::R_LG)),
                ));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inset));
                child.set_clip_rect(inset);
                self.viewport(&mut child, inset, false);
                ui.painter().rect_stroke(
                    inset,
                    CornerRadius::same(theme::R_LG),
                    Stroke::new(1.0, color::LINE_STRONG),
                    egui::StrokeKind::Outside,
                );
            }
            View::Source => self.source_view(ui, body),
        }
    }

    fn viewport(&mut self, ui: &mut Ui, rect: Rect, stats: bool) {
        let camera = self.preview.camera(self.shown_style());
        let list = build_draw_list(&self.sim, &camera);
        self.preview.show(ui, rect, &list, self.shown_style());
        if !stats {
            return;
        }
        let info = format!(
            "{} particles · {} quads{}",
            self.sim.particle_count(),
            list.quads.len(),
            match self.style_view {
                Some(s) if s != self.effect.style => format!(" · restyled to {}", s.label()),
                _ => String::new(),
            }
        );
        ui.painter().text(
            rect.left_top() + Vec2::new(12.0, 10.0),
            egui::Align2::LEFT_TOP,
            info,
            egui::FontId::proportional(11.0),
            color::TEXT_3,
        );
    }

    fn graph(&mut self, ui: &mut Ui, rect: Rect) {
        let mut nodes = Vec::new();
        let mut wires = Vec::new();
        let mut groups = Vec::new();
        for (ei, em) in self.effect.emitters.iter().enumerate() {
            let selected_emitter = self.selection == Selection::Emitter(ei);
            let first = nodes.len();
            let stages: [(Stage, Vec<(String, String, bool)>); 4] = [
                (
                    Stage::Spawn,
                    em.spawn
                        .iter()
                        .map(|m| (m.label().to_string(), spawn_detail(m), !m.is_disabled()))
                        .collect(),
                ),
                (
                    Stage::Init,
                    em.init
                        .iter()
                        .map(|m| (m.label().to_string(), String::new(), !m.is_disabled()))
                        .collect(),
                ),
                (
                    Stage::Update,
                    em.update
                        .iter()
                        .map(|m| (m.label().to_string(), String::new(), !m.is_disabled()))
                        .collect(),
                ),
                (
                    Stage::Render,
                    vec![(
                        render_kind_name(&em.render.kind).to_string(),
                        em.render.texture.label().to_string(),
                        em.enabled,
                    )],
                ),
            ];
            let mut y = 0.0;
            for (si, (stage, rows)) in stages.into_iter().enumerate() {
                let id = (ei as u64) * 16 + si as u64;
                let default = Pos2::new(ei as f32 * 290.0, y);
                let pos = *self.node_pos.entry(id).or_insert(default);
                let rows: Vec<NodeRow> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(ri, (label, detail, enabled))| NodeRow {
                        id: ri as u64,
                        label,
                        detail,
                        enabled,
                        selected: selected_emitter && self.focus == Some((stage, ri)),
                    })
                    .collect();
                let node = Node {
                    id,
                    pos,
                    width: 250.0,
                    title: stage.label().to_string(),
                    subtitle: if si == 0 { em.name.clone() } else { String::new() },
                    rows,
                    selected: selected_emitter && self.focus.is_some_and(|f| f.0 == stage),
                };
                y += node.height() + 36.0;
                nodes.push(node);
                if si > 0 {
                    wires.push(Wire {
                        from: (nodes.len() - 2, Side::Bottom),
                        to: (nodes.len() - 1, Side::Top),
                    });
                }
            }
            groups.push(Group {
                title: em.name.clone(),
                nodes: (first..nodes.len()).collect(),
            });
        }
        let out = node_canvas::node_canvas(ui, rect, &mut self.canvas, &mut nodes, &wires, &groups);
        if out.moved {
            for n in &nodes {
                self.node_pos.insert(n.id, n.pos);
            }
        }
        let stage_of = |id: u64| -> (usize, Stage) {
            let ei = (id / 16) as usize;
            let st = match id % 16 {
                0 => Stage::Spawn,
                1 => Stage::Init,
                2 => Stage::Update,
                _ => Stage::Render,
            };
            (ei, st)
        };
        if let Some(id) = out.clicked_node {
            let (ei, st) = stage_of(id);
            self.select(Selection::Emitter(ei), Some((st, 0)));
        }
        if let Some((id, row)) = out.clicked_row {
            let (ei, st) = stage_of(id);
            self.select(Selection::Emitter(ei), Some((st, row as usize)));
        }
        if let Some((id, row)) = out.toggled_row {
            let (ei, st) = stage_of(id);
            let r = row as usize;
            if let Some(em) = self.effect.emitters.get_mut(ei) {
                match st {
                    Stage::Spawn => {
                        if let Some(m) = em.spawn.get_mut(r) {
                            let d = m.disabled_mut();
                            *d = !*d;
                        }
                    }
                    Stage::Init => {
                        if let Some(m) = em.init.get_mut(r) {
                            let d = m.disabled_mut();
                            *d = !*d;
                        }
                    }
                    Stage::Update => {
                        if let Some(m) = em.update.get_mut(r) {
                            let d = m.disabled_mut();
                            *d = !*d;
                        }
                    }
                    Stage::Render => em.enabled = !em.enabled,
                }
                self.edited();
            }
        }
        if self.effect.emitters.is_empty() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "This effect has no emitters. Trails and beams are edited in the inspector.",
                egui::FontId::proportional(13.0),
                color::TEXT_3,
            );
        }
    }

    fn source_view(&mut self, ui: &mut Ui, rect: Rect) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(12.0)));
        child.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{}  ·  human-readable, one property per line", presets::file_name(&self.effect.name)))
                    .color(color::TEXT_2),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Apply").clicked() {
                    match Effect::from_ron(&self.source) {
                        Ok(e) => {
                            self.effect = e;
                            self.source_error = None;
                            self.clamp_selection();
                            self.dirty = true;
                            match Simulation::new(self.shown(&self.effect), self.seed) {
                                Ok(s) => self.sim = s,
                                Err(err) => self.source_error = Some(err.to_string()),
                            }
                        }
                        Err(e) => self.source_error = Some(e.to_string()),
                    }
                }
                if ui.button("Revert").clicked() {
                    self.source = self.effect.to_ron().unwrap_or_default();
                    self.source_error = None;
                }
            });
        });
        if let Some(err) = &self.source_error {
            child.label(RichText::new(err).color(color::ERROR));
        }
        child.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("source").show(&mut child, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.source)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
    }
}

fn spawn_detail(m: &SpawnModule) -> String {
    match m {
        SpawnModule::Rate { per_second, .. } => format!("{per_second}/s"),
        SpawnModule::Burst { count, time, .. } => format!("{count} at {time}s"),
        SpawnModule::Distance { per_stud, .. } => format!("{per_stud}/stud"),
    }
}

fn render_kind_name(k: &metallic_vfx::RenderKind) -> &'static str {
    match k {
        metallic_vfx::RenderKind::Billboard => "Sprite",
        metallic_vfx::RenderKind::Stretched { .. } => "Stretched sprite",
        metallic_vfx::RenderKind::Ribbon { .. } => "Ribbon",
        metallic_vfx::RenderKind::Mesh { .. } => "Mesh",
    }
}
