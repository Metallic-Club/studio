//! The contextual toolbar (building-tools spec 3.2): a floating bar that follows the selection
//! with the actions people reach for most, so they do not travel to the header or Properties.
//!
//! - **Contents**: the current tool (click cycles Move, Rotate, Scale), its increment, then the
//!   building actions that are registered (colour `colour.open`, material `material.open`, style
//!   `style.open`, Align `build.align`, Mirror `build.mirror`, Array `build.array`; a missing one is
//!   simply not shown, so the bar grows as the tools land) and "More" (Anchored, CanCollide,
//!   Group).
//! - **Placement** ([`place`]): centred on the selection's screen box, 12 px above it; below it
//!   when that is off screen or hits the gizmo; else docked under the viewport header, slid
//!   sideways off the selection. It never covers the selection; when nothing fits it hides.
//! - **When**: after a selection change, once the pointer has been still for 200 ms. Hidden
//!   during drags, camera flight, box selection and Play. A 90 ms fade (instant with reduced
//!   motion).
//! - **Keyboard**: Ctrl+. focuses it, Left and Right move, Enter or Space presses, Esc returns to
//!   the viewport.
//! - View > Contextual toolbar turns it off.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Area, Color32, CornerRadius, Frame, Id, Key, Margin, Order, Pos2, Rect, Sense, Stroke,
    Vec2, pos2, vec2,
};
use metallic_core::{InstanceId, Value};
use metallic_render::glam::DVec3;

use crate::actions::ActionRegistry;
use crate::commands::Command;
use crate::editor::{EditorState, MOVE_STEPS, ROTATE_STEPS, Tool};
use crate::gizmo::AXIS_PX;
use crate::icons::Icon;
use crate::keymap::Keymap;
use crate::theme;
use crate::viewport::Viewport;
use crate::widgets::button::Button;

/// Bar height in px.
pub const HEIGHT: f32 = 36.0;
/// Distance from the selection (and from the viewport edges when sliding) in px.
pub const GAP: f32 = 12.0;
/// How long the pointer must rest after a selection change before the bar shows.
pub const STILL: Duration = Duration::from_millis(200);
/// Fade-in time in seconds (spec 3.2).
pub const FADE: f32 = 0.09;

/// Where [`place`] put the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// 12 px above the selection.
    Above,
    /// 12 px below the selection.
    Below,
    /// Under the viewport header.
    Docked,
}

/// Places a bar of `size` for a selection whose screen box is `selection`, in `viewport` whose
/// usable top is `top` (below the header). `gizmo` is the gizmo's screen box. `None` when no spot
/// leaves the selection uncovered.
pub fn place(
    selection: Rect,
    gizmo: Option<Rect>,
    viewport: Rect,
    top: f32,
    size: Vec2,
) -> Option<(Rect, Placement)> {
    let area = Rect::from_min_max(pos2(viewport.left(), top), viewport.max);
    let left_min = area.left() + GAP;
    let left_max = (area.right() - GAP - size.x).max(left_min);
    let centred = (selection.center().x - size.x / 2.0).clamp(left_min, left_max);
    let fits = |r: Rect| {
        r.top() >= area.top()
            && r.bottom() <= area.bottom()
            && !r.intersects(selection)
            && !gizmo.is_some_and(|g| r.intersects(g))
    };
    let above = Rect::from_min_size(pos2(centred, selection.top() - GAP - size.y), size);
    if fits(above) {
        return Some((above, Placement::Above));
    }
    let below = Rect::from_min_size(pos2(centred, selection.bottom() + GAP), size);
    if fits(below) {
        return Some((below, Placement::Below));
    }
    // Docked under the header: centred if that is clear, else either side; never over the
    // selection or the gizmo.
    let y = area.top() + GAP / 2.0;
    for x in [centred, left_min, left_max] {
        let docked = Rect::from_min_size(pos2(x, y), size);
        if !docked.intersects(selection)
            && !gizmo.is_some_and(|g| docked.intersects(g))
            && docked.bottom() <= area.bottom()
        {
            return Some((docked, Placement::Docked));
        }
    }
    None
}

/// The screen box of the selection's bounding box; `None` when part of it is behind the camera.
pub fn selection_screen_rect(viewport: &Viewport, state: &EditorState) -> Option<Rect> {
    let b = state.selection_bounds()?;
    let (lo, hi) = (b.min, b.max);
    let mut out: Option<Rect> = None;
    for i in 0..8 {
        let p = DVec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        let s = viewport.screen_pos(p)?;
        out = Some(match out {
            Some(r) => r.union(Rect::from_min_max(s, s)),
            None => Rect::from_min_max(s, s),
        });
    }
    out
}

/// The gizmo's screen box (its axes are a constant length on screen), when a tool shows one.
pub fn gizmo_screen_rect(viewport: &Viewport, state: &EditorState) -> Option<Rect> {
    if state.tool == Tool::Select && !state.pivot_edit {
        return None;
    }
    let origin = viewport.screen_pos(state.gizmo_pivot()?.position)?;
    let reach = AXIS_PX as f32 + 16.0;
    Some(Rect::from_center_size(origin, Vec2::splat(reach * 2.0)))
}

/// One button of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Tool,
    Increment,
    Colour,
    Action(&'static str, &'static str, &'static str),
    More,
}

/// The registered building actions, in the spec's order: (action id, label, key).
const ACTIONS: [(&str, &str, &str); 5] = [
    ("material.open", "Material", "M"),
    ("style.open", "Style", "L"),
    ("build.align", "Align", "A"),
    ("build.mirror", "Mirror", "Ctrl+M"),
    ("build.array", "Array", "Ctrl+Shift+D"),
];

/// The contextual toolbar's state.
#[derive(Debug)]
pub struct ContextBar {
    /// View > Contextual toolbar (on by default).
    pub enabled: bool,
    seen_revision: u64,
    armed: bool,
    still_since: Option<Instant>,
    last_pointer: Option<Pos2>,
    focus: Option<usize>,
    /// The focused button still has to take keyboard focus (requested once).
    grab: bool,
    /// Where the bar was drawn last frame (for tests and the hint card).
    pub last_rect: Option<Rect>,
    items: Vec<Item>,
    /// The bar's real width last frame and its item count (placement uses it next frame).
    measured: Option<(usize, f32)>,
}

impl Default for ContextBar {
    fn default() -> Self {
        ContextBar {
            enabled: true,
            seen_revision: u64::MAX,
            armed: false,
            still_since: None,
            last_pointer: None,
            focus: None,
            grab: false,
            last_rect: None,
            items: Vec::with_capacity(ACTIONS.len() + 4),
            measured: None,
        }
    }
}

impl ContextBar {
    /// A bar that is on and waiting for a selection.
    pub fn new() -> ContextBar {
        ContextBar::default()
    }

    /// Shows the bar for selection `revision` without the 200 ms wait (captures and tests).
    pub fn arm(&mut self, revision: u64) {
        self.seen_revision = revision;
        self.armed = true;
    }

    /// Ctrl+.: moves keyboard focus to the bar's first button (when it shows).
    pub fn focus(&mut self) {
        if self.last_rect.is_some() {
            self.focus = Some(0);
            self.grab = true;
        }
    }

    /// True while the bar holds keyboard focus (global keys wait).
    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    /// Shows the bar for this frame (or not). `top` is the bottom of the viewport header.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        state: &mut EditorState,
        viewport: &Viewport,
        actions: &mut ActionRegistry,
        keymap: &Keymap,
        top: f32,
    ) {
        let now = Instant::now();
        let revision = state.selection.revision();
        if revision != self.seen_revision {
            self.seen_revision = revision;
            self.armed = false;
            self.still_since = Some(now);
            self.focus = None;
        }
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        if pointer != self.last_pointer {
            self.last_pointer = pointer;
            if !self.armed {
                self.still_since = Some(now);
            }
        }
        if !self.armed
            && let Some(t) = self.still_since
        {
            let waited = now.duration_since(t);
            if waited >= STILL {
                self.armed = true;
            } else {
                ctx.request_repaint_after(STILL - waited);
            }
        }
        let show = self.enabled
            && self.armed
            && !state.selection.is_empty()
            && !state.play.is_playing()
            && !viewport.is_busy();
        let id = Id::NULL.with("context-bar");
        let alpha = ctx.animate_bool_with_time(id.with("fade"), show, theme::motion(FADE));
        if !show {
            self.last_rect = None;
            self.focus = None;
            return;
        }
        let Some(selection) = selection_screen_rect(viewport, state) else {
            self.last_rect = None;
            return;
        };
        self.collect(actions);
        let width = self.width(state);
        let gizmo = gizmo_screen_rect(viewport, state);
        let Some((rect, _)) = place(selection, gizmo, viewport.rect(), top, vec2(width, HEIGHT))
        else {
            self.last_rect = None;
            return;
        };
        self.last_rect = Some(rect);
        self.keys(ctx);
        let mut run: Option<&'static str> = None;
        let mut held = false;
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                let shown = Frame::new()
                    .fill(theme::bg_raised())
                    .corner_radius(CornerRadius::same(theme::R_MD))
                    .stroke(Stroke::new(1.0, theme::line()))
                    .shadow(theme::SHADOW_POP)
                    .inner_margin(Margin::symmetric(4, 4))
                    .show(ui, |ui| {
                        ui.set_height(HEIGHT - 8.0);
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for i in 0..self.items.len() {
                                let item = self.items[i];
                                let r = self.item(ui, item, state, keymap, &mut run);
                                if self.focus == Some(i) && self.grab {
                                    r.request_focus();
                                }
                                held |= r.has_focus();
                            }
                        });
                    });
                self.measured = Some((self.items.len(), shown.response.rect.width()));
            });
        // Focus is requested once (Ctrl+., Left, Right). When a click elsewhere or another widget
        // takes it, the bar lets go so global shortcuts work again.
        if self.grab {
            self.grab = false;
        } else if self.focus.is_some() && !held {
            self.focus = None;
        }
        if let Some(id) = run {
            actions.run(id, state);
        }
    }

    fn collect(&mut self, actions: &ActionRegistry) {
        self.items.clear();
        self.items.push(Item::Tool);
        self.items.push(Item::Increment);
        if actions.get("colour.open").is_some() {
            self.items.push(Item::Colour);
        }
        for (id, label, key) in ACTIONS {
            if actions.get(id).is_some() {
                self.items.push(Item::Action(id, label, key));
            }
        }
        self.items.push(Item::More);
    }

    /// An estimate of the bar's width, so it can be placed before it is laid out.
    fn width(&self, state: &EditorState) -> f32 {
        if let Some((n, w)) = self.measured
            && n == self.items.len()
        {
            return w;
        }
        let per = |item: &Item| match item {
            Item::Tool => 36.0,
            Item::Increment => {
                if matches!(state.tool, Tool::Rotate) {
                    56.0
                } else {
                    48.0
                }
            }
            Item::Colour => 44.0,
            Item::Action(_, label, key) => 40.0 + 7.0 * (label.len() + key.len()) as f32,
            Item::More => 56.0,
        };
        8.0 + self.items.iter().map(per).sum::<f32>() + 2.0 * self.items.len() as f32
    }

    fn keys(&mut self, ctx: &egui::Context) {
        let Some(f) = self.focus else { return };
        let n = self.items.len();
        let (left, right, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowRight),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if esc {
            self.focus = None;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        } else if right {
            self.focus = Some((f + 1) % n.max(1));
            self.grab = true;
        } else if left {
            self.focus = Some((f + n.max(1) - 1) % n.max(1));
            self.grab = true;
        }
    }

    fn item(
        &mut self,
        ui: &mut egui::Ui,
        item: Item,
        state: &mut EditorState,
        keymap: &Keymap,
        run: &mut Option<&'static str>,
    ) -> egui::Response {
        match item {
            Item::Tool => {
                let (icon, name) = match state.tool {
                    Tool::Rotate => (Icon::Rotate, "Rotate"),
                    Tool::Scale => (Icon::Scale, "Scale"),
                    Tool::Select => (Icon::Select, "Select"),
                    _ => (Icon::Move, "Move"),
                };
                let tip = format!("{name} tool: click for the next of Move, Rotate, Scale");
                let r = ui.add(Button::ghost("").compact().glyph(icon).alt(&tip));
                if r.clicked() {
                    state.tool = match state.tool {
                        Tool::Move => Tool::Rotate,
                        Tool::Rotate => Tool::Scale,
                        _ => Tool::Move,
                    };
                }
                r
            }
            Item::Increment => {
                let rotate = state.tool == Tool::Rotate;
                let (steps, value, unit) = if rotate {
                    (ROTATE_STEPS, state.snapping.rotate_step, "°")
                } else {
                    (MOVE_STEPS, state.snapping.move_step, "")
                };
                let text = format!("{}{unit}", crate::actions::fmt_step(value));
                let tip = if rotate {
                    "Rotate increment"
                } else {
                    "Move increment (studs)"
                };
                let (r, picked) = crate::actions::step_chip(ui, &text, tip, steps, unit, value);
                match picked {
                    Some(step) if rotate => state.snapping.rotate_step = step,
                    Some(step) => state.snapping.move_step = step,
                    None => {}
                }
                r
            }
            Item::Colour => self.swatch(ui, state, keymap, run),
            Item::Action(id, label, key) => {
                let shown = keymap.label(id);
                let key = if shown.is_empty() { key } else { &shown };
                let tip = format!("{label} ({key})");
                let r = ui
                    .add(Button::ghost(label).compact().keycap(key))
                    .on_hover_text(tip);
                if r.clicked() {
                    *run = Some(id);
                }
                r
            }
            Item::More => {
                let r = ui
                    .add(Button::ghost("More").compact())
                    .on_hover_text("Anchored, CanCollide and Group");
                egui::Popup::menu(&r).show(|ui| {
                    for prop in ["Anchored", "CanCollide"] {
                        let (on, mixed) = bool_state(state, prop);
                        let mut value = on;
                        let label = if mixed {
                            format!("{prop} (mixed)")
                        } else {
                            prop.to_string()
                        };
                        if ui.checkbox(&mut value, label).changed() {
                            set_bool(state, prop, value);
                        }
                    }
                    ui.separator();
                    if ui.button("Group").clicked() {
                        *run = Some("edit.group");
                        ui.close();
                    }
                });
                r
            }
        }
    }

    fn swatch(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        keymap: &Keymap,
        run: &mut Option<&'static str>,
    ) -> egui::Response {
        let colours = selection_colours(state);
        let (rect, r) = ui.allocate_exact_size(vec2(40.0, 28.0), Sense::click());
        let key = keymap.label("colour.open");
        let tip = match colours {
            (Some(c), false) => format!(
                "Colour, currently #{:02X}{:02X}{:02X} ({key})",
                c.r(),
                c.g(),
                c.b()
            ),
            _ => format!("Colour: Mixed ({key})"),
        };
        let r = r.on_hover_text(&tip);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
        if r.hovered() || r.has_focus() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::R_SM), theme::bg_hover());
        }
        let square = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        match colours {
            (Some(c), false) => {
                ui.painter().rect_filled(square, 2.0, c);
            }
            (first, _) => {
                // Mixed: a split square (first colour and a neutral half), "Mixed" in the tooltip.
                let a = first.unwrap_or(theme::text_3());
                let mut half = square;
                half.set_right(square.center().x);
                ui.painter().rect_filled(square, 2.0, theme::bg_deep());
                ui.painter().rect_filled(half, 2.0, a);
            }
        }
        ui.painter().rect_stroke(
            square,
            2.0,
            Stroke::new(1.0, theme::line_strong()),
            egui::StrokeKind::Outside,
        );
        if r.clicked() {
            *run = Some("colour.open");
        }
        r
    }
}

/// The selection's part colour: (first colour, mixed).
fn selection_colours(state: &EditorState) -> (Option<Color32>, bool) {
    let mut first: Option<Color32> = None;
    let mut mixed = false;
    for &id in state.selection.ids() {
        if let Ok(Value::Color3(c)) = state.world.get_ref(id, "Color") {
            let c = Color32::from_rgb(c.r as u8, c.g as u8, c.b as u8);
            match first {
                None => first = Some(c),
                Some(f) if f != c => mixed = true,
                _ => {}
            }
        }
    }
    (first, mixed)
}

/// A bool property over the selected parts: (true on the active one, mixed).
fn bool_state(state: &EditorState, prop: &str) -> (bool, bool) {
    let parts = state.selection.parts(&state.world);
    let read = |id: &InstanceId| matches!(state.world.get_ref(*id, prop), Ok(Value::Bool(true)));
    let on = parts.first().is_some_and(read);
    let mixed = parts.iter().any(|id| read(id) != on);
    (on, mixed)
}

/// Sets a bool property on every selected part as one step.
fn set_bool(state: &mut EditorState, prop: &str, value: bool) {
    let parts = state.selection.parts(&state.world);
    let commands: Vec<Command> = parts
        .iter()
        .filter_map(|&id| {
            let old = state.world.get(id, prop).ok()?;
            (old != Value::Bool(value)).then(|| Command::set(id, prop, old, Value::Bool(value)))
        })
        .collect();
    if commands.is_empty() {
        return;
    }
    let n = commands.len();
    let _ = state.apply(Command::Batch {
        name: format!("Set {prop} of {}", crate::commands::count(n, "part")),
        commands,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::EditorCamera;
    use crate::transform::Aabb;

    /// A small deterministic generator (xorshift) for the camera poses.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % 1_000_000) as f64 / 1_000_000.0
        }
    }

    #[test]
    fn the_bar_never_covers_the_selection_in_20_random_camera_poses() {
        let viewport = Rect::from_min_size(pos2(300.0, 76.0), vec2(1000.0, 700.0));
        let top = viewport.top() + 36.0;
        let size = vec2(260.0, HEIGHT);
        let b = Aabb {
            min: DVec3::new(-2.0, 0.0, -1.0),
            max: DVec3::new(2.0, 5.0, 1.0),
        };
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut placed = 0;
        for _ in 0..20 {
            let mut cam = EditorCamera::default();
            cam.rotate(rng.next() * 1200.0 - 600.0, rng.next() * 400.0 - 200.0);
            cam.frame(Aabb {
                min: b.min * (1.0 + rng.next() * 3.0),
                max: b.max * (1.0 + rng.next() * 3.0),
            });
            let view = cam.view((f64::from(viewport.width()), f64::from(viewport.height())));
            let mut sel: Option<Rect> = None;
            for i in 0..8 {
                let p = DVec3::new(
                    if i & 1 == 0 { b.min.x } else { b.max.x },
                    if i & 2 == 0 { b.min.y } else { b.max.y },
                    if i & 4 == 0 { b.min.z } else { b.max.z },
                );
                let (x, y) = view.project(p).expect("in front");
                let s = viewport.min + vec2(x as f32, y as f32);
                sel = Some(sel.map_or(Rect::from_min_max(s, s), |r| {
                    r.union(Rect::from_min_max(s, s))
                }));
            }
            let sel = sel.expect("eight corners");
            let centre = view.project(DVec3::new(0.0, 2.5, 0.0)).expect("pivot");
            let g = viewport.min + vec2(centre.0 as f32, centre.1 as f32);
            let gizmo = Rect::from_center_size(g, Vec2::splat(2.0 * (AXIS_PX as f32 + 16.0)));
            if let Some((r, _)) = place(sel, Some(gizmo), viewport, top, size) {
                placed += 1;
                assert!(!r.intersects(sel), "covers the selection: {r:?} {sel:?}");
                assert!(!r.intersects(gizmo), "covers the gizmo: {r:?} {gizmo:?}");
                assert!(r.top() >= top && r.bottom() <= viewport.bottom());
            }
        }
        assert!(placed >= 15, "placed in {placed} of 20 poses");
    }

    #[test]
    fn above_then_below_then_docked() {
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let size = vec2(200.0, HEIGHT);
        let mid = Rect::from_min_size(pos2(300.0, 300.0), vec2(100.0, 100.0));
        assert_eq!(
            place(mid, None, viewport, 36.0, size).unwrap().1,
            Placement::Above
        );
        let high = Rect::from_min_size(pos2(300.0, 40.0), vec2(100.0, 100.0));
        assert_eq!(
            place(high, None, viewport, 36.0, size).unwrap().1,
            Placement::Below
        );
        let tall = Rect::from_min_size(pos2(300.0, 50.0), vec2(100.0, 540.0));
        let (r, how) = place(tall, None, viewport, 36.0, size).unwrap();
        assert_eq!(how, Placement::Docked);
        assert!(!r.intersects(tall));
        // A gizmo under the header pushes the docked bar to a side, clear of it.
        let gizmo = Rect::from_min_size(pos2(250.0, 36.0), vec2(200.0, 200.0));
        let (r, how) = place(tall, Some(gizmo), viewport, 36.0, size).unwrap();
        assert_eq!(how, Placement::Docked);
        assert!(!r.intersects(tall) && !r.intersects(gizmo), "{r:?}");
        let everything = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        assert!(place(everything, None, viewport, 36.0, size).is_none());
    }
}
