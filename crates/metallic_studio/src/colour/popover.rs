//! The colour popover (design spec `docs/design/building-tools.md`, section 11): a hue ring around
//! a saturation/value square, old/new chip, Hex/RGB/HSV fields, eyedropper, recent colours, saved
//! palettes and an OKLCh harmony row.
//!
//! The popover never edits the scene itself. [`ColourPopover::show`] returns [`ColourEvent`]s:
//! previews while a gesture runs and one commit per gesture, which the editor turns into its
//! preview overlay and one undo step (wired up with the editor, #10 and #213).

use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Mesh, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, accesskit, pos2, vec2,
};

use super::maths::{Harmony, Hsv, Rgb8, eval_number, nearest_css_name, parse_colour, undo_name};
use super::palette::{PaletteStore, RecentColours};
use crate::theme;

/// Popover width (spec 11.1).
pub const WIDTH: f32 = 320.0;
/// Popover height (spec 11.1).
pub const HEIGHT: f32 = 476.0;
/// Hue ring outer diameter.
pub const RING: f32 = 248.0;
/// Hue ring thickness.
pub const RING_THICKNESS: f32 = 18.0;
/// Saturation/value square side.
pub const SQUARE: f32 = 146.0;
/// Keyboard and typed edits commit after this pause (seconds).
pub const COMMIT_PAUSE: f64 = 0.6;

const PAD: f32 = 12.0;
const ROW: f32 = 24.0;
const GAP: f32 = 4.0;
const LABEL_W: f32 = 52.0;
const SWATCH: f32 = 16.0;
const SWATCH_GAP: f32 = 4.0;
const HEADER: f32 = 22.0;

/// What the popover asks the editor to do.
#[derive(Debug, Clone, PartialEq)]
pub enum ColourEvent {
    /// Show this colour on every target through the preview overlay (no undo step yet).
    Preview(Rgb8),
    /// Alt-drag on the ring: preview each target's own colour turned by this many degrees.
    PreviewHueShift(f32),
    /// Commit one undo step setting every target to `colour`.
    Commit {
        /// The colour.
        colour: Rgb8,
        /// Undo name, "Colour: #5280CC (3 parts)".
        undo_name: String,
    },
    /// Commit one undo step turning each target's hue by `degrees`.
    CommitHueShift {
        /// Hue change in degrees, -180 to 180.
        degrees: f32,
        /// Undo name.
        undo_name: String,
    },
    /// Esc: drop the uncommitted preview. Committed steps stay. The popover is closed.
    Cancel,
    /// The popover closed after committing anything pending.
    Closed,
    /// The eyedropper is armed and the user clicked outside the popover. The editor answers
    /// with [`ColourPopover::pick`] (a part's `Color`) or [`ColourPopover::pick_screen_pixel`].
    EyedropperClick {
        /// Where, in points.
        pos: Pos2,
        /// Alt held: pick the rendered pixel, not the property.
        alt: bool,
        /// Ctrl held: pick colour, material and style together and paint them (spec 11.3).
        ctrl: bool,
    },
}

/// The lists the popover reads and edits; they outlive any one popover.
#[derive(Debug, Clone, Default)]
pub struct ColourLibrary {
    /// Recent colours (per user).
    pub recent: RecentColours,
    /// Saved palettes (per project).
    pub palettes: PaletteStore,
    /// Which palette the palette row shows.
    pub active_palette: usize,
    /// Which harmony rule the harmony row uses.
    pub harmony: Harmony,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Hex,
    R,
    G,
    B,
    H,
    S,
    V,
}

impl Field {
    const ALL: [Field; 7] = [
        Field::Hex,
        Field::R,
        Field::G,
        Field::B,
        Field::H,
        Field::S,
        Field::V,
    ];

    fn prefix(self) -> &'static str {
        match self {
            Field::Hex => "#",
            Field::R => "R",
            Field::G => "G",
            Field::B => "B",
            Field::H => "H",
            Field::S => "S",
            Field::V => "V",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Field::Hex => "Hex",
            Field::R => "Red",
            Field::G => "Green",
            Field::B => "Blue",
            Field::H => "Hue, degrees",
            Field::S => "Saturation, percent",
            Field::V => "Value, percent",
        }
    }

    fn max(self) -> f64 {
        match self {
            Field::R | Field::G | Field::B => 255.0,
            Field::H => 359.0,
            Field::S | Field::V | Field::Hex => 100.0,
        }
    }

    fn value(self, hsv: Hsv) -> f64 {
        let rgb = hsv.to_rgb8();
        match self {
            Field::R => f64::from(rgb.r),
            Field::G => f64::from(rgb.g),
            Field::B => f64::from(rgb.b),
            Field::H => f64::from(hue_degrees(hsv)),
            Field::S => f64::from((hsv.s * 100.0).round()),
            Field::V => f64::from((hsv.v * 100.0).round()),
            Field::Hex => 0.0,
        }
    }

    fn format(self, hsv: Hsv) -> String {
        match self {
            Field::Hex => hsv.to_rgb8().to_hex()[1..].to_owned(),
            _ => format!("{}", self.value(hsv)),
        }
    }

    /// `hsv` with this field set to `v` (clamped). RGB fields keep the hue for greys.
    fn apply(self, hsv: Hsv, v: f64) -> Hsv {
        let v = v.clamp(0.0, self.max()) as f32;
        let byte = v.round() as u8;
        let rgb = hsv.to_rgb8();
        match self {
            Field::R => Hsv::from_rgb8(Rgb8 { r: byte, ..rgb }, hsv.h),
            Field::G => Hsv::from_rgb8(Rgb8 { g: byte, ..rgb }, hsv.h),
            Field::B => Hsv::from_rgb8(Rgb8 { b: byte, ..rgb }, hsv.h),
            Field::H => Hsv::new(v, hsv.s, hsv.v),
            Field::S => Hsv::new(hsv.h, v / 100.0, hsv.v),
            Field::V => Hsv::new(hsv.h, hsv.s, v / 100.0),
            Field::Hex => hsv,
        }
    }
}

/// Hue as a whole number of degrees, 0 to 359.
fn hue_degrees(hsv: Hsv) -> i32 {
    (hsv.h.round() as i32).rem_euclid(360)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragKind {
    Ring,
    Square,
    /// Alt-drag on the ring: shift each target's hue by how far the pointer has turned since
    /// the press. `base_hue` is the shown hue at the press, `press_hue` the pointer's angle then,
    /// `delta` the shift so far (degrees, -180 to 180).
    RingShift {
        base_hue: f32,
        press_hue: f32,
        delta: f32,
    },
    Scrub {
        field: Field,
        start: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Recent,
    Palette,
    Harmony,
}

impl Row {
    fn index(self) -> usize {
        self as usize
    }
}

/// The colour popover. Keep one per Studio window and call [`ColourPopover::show`] every frame.
pub struct ColourPopover {
    id: Id,
    open: bool,
    pos: Pos2,
    rect: Rect,
    wheel_centre: Pos2,
    hsv: Hsv,
    original: Rgb8,
    last_committed: Rgb8,
    last_previewed: Option<Rgb8>,
    targets: usize,
    mixed: bool,
    touched: bool,
    pending_since: Option<f64>,
    drag: Option<DragKind>,
    texts: [String; 7],
    eyedropper: bool,
    screen_pick: Option<Arc<Mutex<Option<Rgb8>>>>,
    roving: [usize; 3],
    palette_drag: Option<usize>,
    renaming: Option<String>,
    message: Option<String>,
    /// Frames left in which to move focus to the ring (an Area is invisible on its first frame).
    focus_on_open: u8,
    events: Vec<ColourEvent>,
}

impl ColourPopover {
    /// A closed popover. `id_salt` must be unique among popovers in one window.
    pub fn new(id_salt: &str) -> Self {
        Self {
            id: Id::NULL.with(("colour_popover", id_salt)),
            open: false,
            pos: Pos2::ZERO,
            rect: Rect::NOTHING,
            wheel_centre: Pos2::ZERO,
            hsv: Hsv::default(),
            original: Rgb8::default(),
            last_committed: Rgb8::default(),
            last_previewed: None,
            targets: 1,
            mixed: false,
            touched: false,
            pending_since: None,
            drag: None,
            texts: Default::default(),
            eyedropper: false,
            screen_pick: None,
            roving: [0; 3],
            palette_drag: None,
            renaming: None,
            message: None,
            focus_on_open: 0,
            events: Vec::new(),
        }
    }

    /// Opens on `colour` (the active object's), for `targets` objects; `mixed` when the targets
    /// differ. `pos` is the popover's top-left corner. Keyboard focus starts on the ring.
    pub fn open(&mut self, colour: Rgb8, targets: usize, mixed: bool, pos: Pos2) {
        self.open = true;
        self.pos = pos;
        self.hsv = Hsv::from_rgb8(colour, self.hsv.h);
        self.original = colour;
        self.last_committed = colour;
        self.last_previewed = None;
        self.targets = targets.max(1);
        self.mixed = mixed;
        self.touched = false;
        self.pending_since = None;
        self.drag = None;
        self.eyedropper = false;
        self.screen_pick = None;
        self.renaming = None;
        self.message = None;
        self.focus_on_open = 4;
    }

    /// True while shown.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The colour now shown as "new".
    pub fn colour(&self) -> Rgb8 {
        self.hsv.to_rgb8()
    }

    /// Hue, saturation and value as shown (hue survives greys).
    pub fn hsv(&self) -> Hsv {
        self.hsv
    }

    /// True while the eyedropper waits for a click.
    pub fn eyedropper_armed(&self) -> bool {
        self.eyedropper
    }

    /// The popover's rectangle last frame (for hit tests by the editor).
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The centre of the hue ring and square last frame.
    pub fn wheel_centre(&self) -> Pos2 {
        self.wheel_centre
    }

    /// Arms or disarms the eyedropper (the I key, or the pipette button).
    pub fn set_eyedropper(&mut self, armed: bool) {
        self.eyedropper = armed;
    }

    /// The eyedropper result from the editor (a part's `Color`): applies it as one step.
    pub fn pick(&mut self, colour: Rgb8, library: &mut ColourLibrary) {
        self.eyedropper = false;
        self.set_hsv(Hsv::from_rgb8(colour, self.hsv.h));
        self.commit(library);
    }

    /// Picks the Studio window's pixel at `pos` (points). The result arrives a frame or two
    /// later through a screenshot of the window and is applied as one step.
    pub fn pick_screen_pixel(&mut self, ctx: &egui::Context, pos: Pos2) {
        let slot = Arc::new(Mutex::new(None));
        let writer = Arc::clone(&slot);
        let ppp = ctx.pixels_per_point();
        ctx.request_screenshot(move |image| {
            let picked = pixel_at(&image, pos, ppp);
            if let Ok(mut s) = writer.lock() {
                *s = picked;
            }
        });
        self.screen_pick = Some(slot);
        self.eyedropper = false;
    }

    /// Closes, committing anything pending (focus leaving counts as a commit, spec 11.2).
    pub fn close(&mut self, library: &mut ColourLibrary) {
        if !self.open {
            return;
        }
        self.finish_gesture(library);
        self.commit(library);
        self.open = false;
        self.eyedropper = false;
        self.events.push(ColourEvent::Closed);
    }

    fn cancel(&mut self) {
        self.open = false;
        self.eyedropper = false;
        self.drag = None;
        self.pending_since = None;
        self.hsv = Hsv::from_rgb8(self.last_committed, self.hsv.h);
        self.events.push(ColourEvent::Cancel);
    }

    fn set_hsv(&mut self, hsv: Hsv) {
        self.hsv = hsv;
        self.touched = true;
        let c = hsv.to_rgb8();
        if self.last_previewed != Some(c) {
            self.last_previewed = Some(c);
            self.events.push(ColourEvent::Preview(c));
        }
    }

    /// A keyboard or typed change: preview now, commit after a pause.
    fn edit_by_keyboard(&mut self, hsv: Hsv, now: f64) {
        self.set_hsv(hsv);
        self.pending_since = Some(now);
    }

    fn commit(&mut self, library: &mut ColourLibrary) {
        self.pending_since = None;
        let c = self.colour();
        if c == self.last_committed {
            return;
        }
        self.last_committed = c;
        library.recent.push(c);
        self.events.push(ColourEvent::Commit {
            colour: c,
            undo_name: undo_name(c, self.targets),
        });
    }

    fn finish_gesture(&mut self, library: &mut ColourLibrary) {
        match self.drag.take() {
            Some(DragKind::RingShift { delta, .. }) => {
                let degrees = delta;
                if degrees.round() == 0.0 {
                    // No net turn: no undo step. Clear any preview a back-and-forth left behind.
                    if delta != 0.0 {
                        self.events.push(ColourEvent::PreviewHueShift(0.0));
                    }
                    return;
                }
                let noun = if self.targets == 1 { "part" } else { "parts" };
                self.events.push(ColourEvent::CommitHueShift {
                    degrees,
                    undo_name: format!("Hue shift: {degrees:+.0}° ({} {noun})", self.targets),
                });
                self.last_committed = self.colour();
            }
            Some(_) => self.commit(library),
            None => {}
        }
    }

    /// Shows the popover (when open) and returns what happened this frame.
    pub fn show(&mut self, ctx: &egui::Context, library: &mut ColourLibrary) -> Vec<ColourEvent> {
        if self.open {
            self.poll_screen_pick(library);
            let now = ctx.input(|i| i.time);
            self.trap_focus(ctx);
            let area = egui::Area::new(self.id)
                .order(egui::Order::Foreground)
                .fixed_pos(self.pos)
                .constrain(true);
            let response = area.show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(PAD)
                    .show(ui, |ui| self.contents(ui, library, now));
            });
            self.rect = response.response.rect;
            self.after_frame(ctx, library, now);
        }
        std::mem::take(&mut self.events)
    }

    fn poll_screen_pick(&mut self, library: &mut ColourLibrary) {
        let picked = self
            .screen_pick
            .as_ref()
            .and_then(|slot| slot.lock().ok().and_then(|mut s| s.take()));
        if let Some(c) = picked {
            self.screen_pick = None;
            self.pick(c, library);
        }
    }

    /// Tab from the last control wraps to the first and Shift+Tab from the first to the last,
    /// so focus stays inside until Esc (spec 14.3).
    fn trap_focus(&mut self, ctx: &egui::Context) {
        let (first, last) = (self.id.with("ring"), self.id.with("close"));
        if self.focus_on_open > 0 {
            self.focus_on_open -= 1;
            ctx.memory_mut(|m| m.request_focus(first));
            return;
        }
        let (tab, shift) = ctx.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift));
        if !tab {
            return;
        }
        let focused = ctx.memory(|m| m.focused());
        let target = match (focused, shift) {
            (Some(f), false) if f == last => Some(first),
            (Some(f), true) if f == first => Some(last),
            _ => None,
        };
        if let Some(t) = target {
            ctx.memory_mut(|m| {
                m.move_focus(egui::FocusDirection::None);
                m.request_focus(t);
            });
        }
    }

    fn after_frame(&mut self, ctx: &egui::Context, library: &mut ColourLibrary, now: f64) {
        if !self.open {
            return;
        }
        let (esc, pointer_down, clicked, pos, mods) = ctx.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.pointer.any_down(),
                i.pointer.primary_clicked(),
                i.pointer.interact_pos(),
                i.modifiers,
            )
        });
        if self.drag.is_some() && !pointer_down {
            self.finish_gesture(library);
        }
        if esc && self.renaming.take().is_none() {
            if self.eyedropper {
                self.eyedropper = false;
            } else if !egui::Popup::is_any_open(ctx) {
                self.cancel();
                return;
            }
        }
        if self.eyedropper {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
            if clicked && let Some(p) = pos.filter(|p| !self.rect.contains(*p)) {
                self.events.push(ColourEvent::EyedropperClick {
                    pos: p,
                    alt: mods.alt,
                    ctrl: mods.ctrl || mods.command,
                });
            }
        }
        if let Some(since) = self.pending_since {
            let left = since + COMMIT_PAUSE - now;
            if left <= 0.0 {
                self.commit(library);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
            }
        }
        if self.screen_pick.is_some() {
            ctx.request_repaint();
        }
    }

    fn contents(&mut self, ui: &mut Ui, library: &mut ColourLibrary, now: f64) {
        // The frame's 1 px stroke sits outside the margin, so the content is 2 px smaller.
        let inner_w = WIDTH - 2.0 * PAD - 2.0;
        ui.set_width(inner_w);
        ui.set_min_height(HEIGHT - 2.0 * PAD - 2.0);
        ui.spacing_mut().item_spacing = vec2(6.0, GAP);

        // Header: the buttons are placed here but created last, for the tab order.
        let (header, _) = ui.allocate_exact_size(vec2(inner_w, HEADER), Sense::hover());
        let painter = ui.painter();
        let title = painter.text(
            header.left_center(),
            Align2::LEFT_CENTER,
            "Colour",
            FontId::proportional(14.0),
            theme::TEXT,
        );
        let (sub, sub_colour) = match &self.message {
            Some(m) => (m.clone(), theme::ERROR),
            None => (
                format!(
                    "{} {}",
                    self.targets,
                    if self.targets == 1 { "part" } else { "parts" }
                ),
                theme::TEXT_DIM,
            ),
        };
        painter.text(
            pos2(title.right() + 8.0, header.center().y),
            Align2::LEFT_CENTER,
            sub,
            FontId::proportional(12.0),
            sub_colour,
        );
        let close_rect =
            Rect::from_center_size(header.right_center() - vec2(10.0, 0.0), Vec2::splat(22.0));
        let pipette_rect = close_rect.translate(vec2(-30.0, 0.0));

        let (wheel, _) = ui.allocate_exact_size(vec2(inner_w, RING), Sense::hover());
        let centre = wheel.center();
        self.wheel_centre = centre;
        self.ring(ui, centre, now);
        self.square(ui, centre, now);

        self.chip_and_hex_row(ui, library, now);
        self.number_row(ui, "RGB", [Field::R, Field::G, Field::B], library, now);
        self.number_row(ui, "HSV", [Field::H, Field::S, Field::V], library, now);

        let pipette = self.icon_button(ui, pipette_rect, "pipette", Icon::Pipette, self.eyedropper);
        pipette
            .clone()
            .on_hover_text("Eyedropper (I). Alt picks the rendered pixel.");
        if pipette.clicked() {
            self.eyedropper = !self.eyedropper;
        }
        if ui.input(|i| i.key_pressed(Key::I) && i.modifiers.is_none())
            && !ui.ctx().text_edit_focused()
        {
            self.eyedropper = !self.eyedropper;
        }

        self.recent_row(ui, library);
        self.palette_row(ui, library);
        self.harmony_row(ui, library);

        let close = self.icon_button(ui, close_rect, "close", Icon::Close, false);
        if close
            .on_hover_text("Close (Esc cancels the preview)")
            .clicked()
        {
            self.close(library);
        }
    }

    fn ring(&mut self, ui: &mut Ui, centre: Pos2, now: f64) {
        let id = self.id.with("ring");
        let outer = RING / 2.0;
        let inner = outer - RING_THICKNESS;
        let rect = Rect::from_center_size(centre, Vec2::splat(RING));
        let resp = ui.interact(rect, id, Sense::click_and_drag());

        // Pointer: only presses that land on the band start a ring drag.
        if resp.is_pointer_button_down_on()
            && let Some(p) = resp.interact_pointer_pos()
        {
            if self.drag.is_none() {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
                let d = origin.distance(centre);
                if (inner - 6.0..=outer + 6.0).contains(&d) {
                    resp.request_focus();
                    self.drag = Some(if ui.input(|i| i.modifiers.alt) {
                        DragKind::RingShift {
                            base_hue: self.hsv.h,
                            press_hue: ring_hue(origin - centre),
                            delta: 0.0,
                        }
                    } else {
                        DragKind::Ring
                    });
                }
            }
            if matches!(self.drag, Some(DragKind::Ring | DragKind::RingShift { .. })) {
                let hue = ring_hue(p - centre);
                if let Some(DragKind::RingShift {
                    base_hue,
                    press_hue,
                    delta,
                }) = self.drag
                {
                    let new_delta = signed_degrees(hue - press_hue);
                    if new_delta != delta {
                        self.drag = Some(DragKind::RingShift {
                            base_hue,
                            press_hue,
                            delta: new_delta,
                        });
                        self.hsv = Hsv::new(base_hue + new_delta, self.hsv.s, self.hsv.v);
                        self.touched = true;
                        self.events.push(ColourEvent::PreviewHueShift(new_delta));
                    }
                } else {
                    self.set_hsv(Hsv::new(hue, self.hsv.s, self.hsv.v));
                }
            }
        }

        // Keyboard (WAI-ARIA slider keys, spec 11.4).
        if resp.has_focus() {
            self.focus_on_open = 0;
            lock_arrows(ui, id);
            let h = hue_degrees(self.hsv) as f32;
            let step = |i: &egui::InputState| if i.modifiers.shift { 10.0 } else { 1.0 };
            let new_h = ui.input(|i| {
                if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp) {
                    Some(h + step(i))
                } else if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown) {
                    Some(h - step(i))
                } else if i.key_pressed(Key::PageUp) {
                    Some(h + 30.0)
                } else if i.key_pressed(Key::PageDown) {
                    Some(h - 30.0)
                } else if i.key_pressed(Key::Home) {
                    Some(0.0)
                } else if i.key_pressed(Key::End) {
                    Some(359.0)
                } else {
                    None
                }
            });
            let ak = ui.input(|i| {
                i.num_accesskit_action_requests(id, accesskit::Action::Increment) as f32
                    - i.num_accesskit_action_requests(id, accesskit::Action::Decrement) as f32
            });
            let new_h = new_h.or((ak != 0.0).then_some(h + ak));
            if let Some(nh) = new_h {
                self.edit_by_keyboard(Hsv::new(nh, self.hsv.s, self.hsv.v), now);
            }
        }

        // Paint the band.
        let painter = ui.painter();
        let mut mesh = Mesh::default();
        const SEGMENTS: u32 = 180;
        for i in 0..=SEGMENTS {
            let hue = i as f32 * 360.0 / SEGMENTS as f32;
            let a = (hue - 90.0).to_radians();
            let dir = vec2(a.cos(), a.sin());
            let c = to_color32(Hsv::new(hue, 1.0, 1.0).to_rgb8());
            mesh.colored_vertex(centre + dir * inner, c);
            mesh.colored_vertex(centre + dir * outer, c);
            if i > 0 {
                let b = 2 * i;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b + 1, b);
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.circle_stroke(centre, outer, Stroke::new(1.0, theme::BORDER));
        painter.circle_stroke(centre, inner, Stroke::new(1.0, theme::BORDER));
        if resp.has_focus() {
            painter.circle_stroke(centre, outer + 3.0, Stroke::new(2.0, theme::FOCUS));
        }
        let a = (self.hsv.h - 90.0).to_radians();
        let mid = centre + vec2(a.cos(), a.sin()) * (outer - RING_THICKNESS / 2.0);
        paint_marker(
            painter,
            mid,
            8.0,
            to_color32(Hsv::new(self.hsv.h, 1.0, 1.0).to_rgb8()),
        );

        let deg = hue_degrees(self.hsv);
        let value = format!(
            "{deg} degrees, close to {}",
            nearest_css_name(self.colour())
        );
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(accesskit::Role::Slider);
            node.set_label("Hue");
            node.set_value(value.clone());
            node.set_numeric_value(f64::from(deg));
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(359.0);
            node.set_numeric_value_step(1.0);
            node.add_action(accesskit::Action::Increment);
            node.add_action(accesskit::Action::Decrement);
        });
    }

    fn square(&mut self, ui: &mut Ui, centre: Pos2, now: f64) {
        let id = self.id.with("square");
        let rect = Rect::from_center_size(centre, Vec2::splat(SQUARE));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if resp.is_pointer_button_down_on()
            && let Some(p) = resp.interact_pointer_pos()
        {
            if self.drag.is_none() {
                resp.request_focus();
                self.drag = Some(DragKind::Square);
            }
            if self.drag == Some(DragKind::Square) {
                let s = (p.x - rect.left()) / rect.width();
                let v = 1.0 - (p.y - rect.top()) / rect.height();
                self.set_hsv(Hsv::new(self.hsv.h, s, v));
            }
        }

        // Keyboard (2D colour area keys, spec 11.4).
        if resp.has_focus() {
            lock_arrows(ui, id);
            let s = (self.hsv.s * 100.0).round();
            let v = (self.hsv.v * 100.0).round();
            let change = ui.input(|i| {
                let k = if i.modifiers.shift { 10.0 } else { 1.0 };
                if i.key_pressed(Key::ArrowRight) {
                    Some((s + k, v))
                } else if i.key_pressed(Key::ArrowLeft) {
                    Some((s - k, v))
                } else if i.key_pressed(Key::ArrowUp) {
                    Some((s, v + k))
                } else if i.key_pressed(Key::ArrowDown) {
                    Some((s, v - k))
                } else if i.key_pressed(Key::PageUp) {
                    Some((s, v + 10.0))
                } else if i.key_pressed(Key::PageDown) {
                    Some((s, v - 10.0))
                } else if i.key_pressed(Key::Home) {
                    Some((s - 10.0, v))
                } else if i.key_pressed(Key::End) {
                    Some((s + 10.0, v))
                } else {
                    None
                }
            });
            if let Some((ns, nv)) = change {
                self.edit_by_keyboard(Hsv::new(self.hsv.h, ns / 100.0, nv / 100.0), now);
            }
        }

        // Paint: a grid with exact colours at each vertex.
        let painter = ui.painter();
        const N: u32 = 24;
        let mut mesh = Mesh::default();
        for row in 0..=N {
            for col in 0..=N {
                let (x, y) = (col as f32 / N as f32, row as f32 / N as f32);
                let c = Hsv::new(self.hsv.h, x, 1.0 - y).to_rgb8();
                mesh.colored_vertex(
                    rect.min + vec2(x * rect.width(), y * rect.height()),
                    to_color32(c),
                );
                if row > 0 && col > 0 {
                    let i = row * (N + 1) + col;
                    let (a, b, c2, d) = (i - N - 2, i - N - 1, i - 1, i);
                    mesh.add_triangle(a, b, d);
                    mesh.add_triangle(a, d, c2);
                }
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Outside,
        );
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                2.0,
                Stroke::new(2.0, theme::FOCUS),
                StrokeKind::Outside,
            );
        }
        let marker = rect.min
            + vec2(
                self.hsv.s * rect.width(),
                (1.0 - self.hsv.v) * rect.height(),
            );
        paint_marker(painter, marker, 6.0, to_color32(self.colour()));

        let (sp, vp) = ((self.hsv.s * 100.0).round(), (self.hsv.v * 100.0).round());
        let value = format!(
            "Saturation {sp} percent, value {vp} percent, close to {}",
            nearest_css_name(self.colour())
        );
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(accesskit::Role::Slider);
            node.set_label("Saturation and value");
            node.set_value(value.clone());
            node.set_numeric_value(f64::from(sp));
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(100.0);
        });
    }

    fn chip_and_hex_row(&mut self, ui: &mut Ui, library: &mut ColourLibrary, now: f64) {
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            let id = self.id.with("chip");
            let (rect, _) = ui.allocate_exact_size(vec2(64.0, ROW), Sense::hover());
            let resp = ui.interact(rect, id, Sense::click());
            let (old, new) = rect.split_left_right_at_fraction(0.5);
            let painter = ui.painter();
            painter.rect_filled(
                old,
                CornerRadius {
                    nw: 4,
                    sw: 4,
                    ne: 0,
                    se: 0,
                },
                to_color32(self.original),
            );
            painter.rect_filled(
                new,
                CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: 4,
                    se: 4,
                },
                to_color32(self.colour()),
            );
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, theme::BORDER),
                StrokeKind::Inside,
            );
            if resp.has_focus() {
                focus_ring(painter, rect, 4.0);
            }
            let restore =
                resp.clicked() && resp.interact_pointer_pos().is_none_or(|p| old.contains(p));
            if restore {
                self.set_hsv(Hsv::from_rgb8(self.original, self.hsv.h));
                self.commit(library);
            }
            resp.on_hover_text("Left: colour when opened (click to restore). Right: new colour.");
            let original = self.original.to_hex();
            ui.ctx().accesskit_node_builder(id, |node| {
                node.set_role(accesskit::Role::Button);
                node.set_label(format!("Restore original colour {original}"));
            });

            if self.mixed && !self.touched {
                ui.label(
                    egui::RichText::new("Mixed")
                        .size(12.0)
                        .color(theme::TEXT_DIM),
                );
            } else {
                ui.add_space(2.0);
            }
            ui.label(egui::RichText::new("Hex").size(13.0).color(theme::TEXT_DIM));
            let w = ui.available_width();
            self.field(ui, Field::Hex, w, library, now);
        });
    }

    fn number_row(
        &mut self,
        ui: &mut Ui,
        label: &str,
        fields: [Field; 3],
        library: &mut ColourLibrary,
        now: f64,
    ) {
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            row_label(ui, label);
            let w = (ui.available_width() - 2.0 * 6.0) / 3.0;
            for f in fields {
                self.field(ui, f, w, library, now);
            }
        });
    }

    /// One labelled number (or hex) field: scrub by dragging the letter, type a value or an
    /// expression, Enter commits.
    fn field(
        &mut self,
        ui: &mut Ui,
        field: Field,
        width: f32,
        library: &mut ColourLibrary,
        now: f64,
    ) {
        let index = Field::ALL.iter().position(|f| *f == field).unwrap_or(0);
        let edit_id = self.id.with(("field", index));
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
        let has_focus = ui.memory(|m| m.has_focus(edit_id));
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 4.0, theme::DEEP);
        let hovered = ui.rect_contains_pointer(rect);
        let border = if hovered {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, border), StrokeKind::Inside);

        // The letter is the scrub handle (not a tab stop: the field is).
        let handle = Rect::from_min_size(rect.min, vec2(18.0, ROW));
        painter.text(
            handle.center(),
            Align2::CENTER_CENTER,
            field.prefix(),
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        if field != Field::Hex {
            let scrub = ui
                .interact(handle, self.id.with(("scrub", index)), Sense::DRAG)
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
            if scrub.drag_started() {
                self.drag = Some(DragKind::Scrub {
                    field,
                    start: field.value(self.hsv),
                });
            }
            if scrub.dragged()
                && let Some(DragKind::Scrub { field: f, start }) = self.drag
                && f == field
            {
                let total = scrub.total_drag_delta().map_or(0.0, |d| d.x);
                let v = start + f64::from(total) * 0.5;
                self.set_hsv(field.apply(self.hsv, v.round()));
            }
        }

        if !has_focus {
            self.texts[index] = field.format(self.hsv);
        }
        let text_rect =
            Rect::from_min_max(pos2(handle.right(), rect.top()), rect.max - vec2(6.0, 0.0));
        let resp = ui.put(
            text_rect,
            TextEdit::singleline(&mut self.texts[index])
                .id(edit_id)
                .frame(egui::Frame::NONE)
                .align(Align2::RIGHT_CENTER)
                .font(egui::TextStyle::Monospace)
                .char_limit(if field == Field::Hex { 24 } else { 12 })
                .desired_width(text_rect.width()),
        );
        if resp.gained_focus() {
            select_all(ui, edit_id, &self.texts[index]);
        }
        if resp.has_focus() {
            focus_ring(&painter, rect, 4.0);
        }
        if resp.changed() {
            let text = self.texts[index].clone();
            let parsed = match field {
                Field::Hex => parse_colour(&text)
                    .ok()
                    .map(|c| Hsv::from_rgb8(c, self.hsv.h)),
                _ => eval_number(&text).map(|v| field.apply(self.hsv, v)),
            };
            if let Some(hsv) = parsed {
                self.edit_by_keyboard(hsv, now);
            }
        }
        if resp.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                self.commit(library);
            }
            self.texts[index] = field.format(self.hsv);
        }
        let name = field.name();
        ui.ctx().accesskit_node_builder(edit_id, |node| {
            node.set_label(name);
        });
    }

    fn swatch(
        &mut self,
        ui: &mut Ui,
        row: Row,
        index: usize,
        colour: Rgb8,
        extra: Sense,
    ) -> Response {
        let id = self.id.with(("swatch", row.index(), index));
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(SWATCH), Sense::hover());
        let focusable = self.roving[row.index()] == index;
        let sense = if focusable {
            Sense::click() | extra
        } else {
            Sense::CLICK | extra
        };
        let resp = ui.interact(rect, id, sense);
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, to_color32(colour));
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );
        if colour == self.colour() {
            painter.rect_stroke(
                rect.expand(1.0),
                4.0,
                Stroke::new(1.0, theme::TEXT),
                StrokeKind::Outside,
            );
        }
        if resp.hovered() {
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, theme::ACCENT),
                StrokeKind::Inside,
            );
        }
        if resp.has_focus() {
            focus_ring(painter, rect, 3.0);
        }
        let hex = colour.to_hex();
        let name = nearest_css_name(colour);
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(accesskit::Role::Button);
            node.set_label(format!("{hex}, close to {name}"));
        });
        if resp.clicked() {
            self.roving[row.index()] = index;
        }
        resp.on_hover_text(hex)
    }

    /// Arrow keys move along a swatch row (one tab stop per row). Returns the key's direction.
    fn roving_keys(&mut self, ui: &Ui, row: Row, focused: Option<usize>, len: usize) {
        let Some(i) = focused else { return };
        lock_arrows(ui, self.id.with(("swatch", row.index(), i)));
        let (left, right, ctrl) = ui.input(|inp| {
            (
                inp.key_pressed(Key::ArrowLeft),
                inp.key_pressed(Key::ArrowRight),
                inp.modifiers.ctrl || inp.modifiers.command,
            )
        });
        if ctrl {
            return;
        }
        let next = if right && i + 1 < len {
            i + 1
        } else if left && i > 0 {
            i - 1
        } else {
            return;
        };
        self.roving[row.index()] = next;
        let id = self.id.with(("swatch", row.index(), next));
        ui.memory_mut(|m| m.request_focus(id));
    }

    fn recent_row(&mut self, ui: &mut Ui, library: &mut ColourLibrary) {
        let colours = library.recent.items().to_vec();
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            ui.spacing_mut().item_spacing.x = SWATCH_GAP;
            row_label(ui, "Recent");
            if colours.is_empty() {
                ui.label(
                    egui::RichText::new("Committed colours appear here")
                        .size(12.0)
                        .color(theme::TEXT_DIM),
                );
            }
            self.roving[0] = self.roving[0].min(colours.len().saturating_sub(1));
            let mut focused = None;
            for (i, c) in colours.iter().enumerate() {
                let resp = self.swatch(ui, Row::Recent, i, *c, Sense::hover());
                if resp.clicked() {
                    self.set_hsv(Hsv::from_rgb8(*c, self.hsv.h));
                    self.commit(library);
                }
                if resp.has_focus() {
                    focused = Some(i);
                }
            }
            self.roving_keys(ui, Row::Recent, focused, colours.len());
        });
    }

    fn palette_row(&mut self, ui: &mut Ui, library: &mut ColourLibrary) {
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            row_label(ui, "Palette");
            let count = library.palettes.palettes().len();
            library.active_palette = library.active_palette.min(count.saturating_sub(1));
            let active = library.active_palette;
            let current_name = library
                .palettes
                .get(active)
                .map_or_else(|| "None".to_owned(), |p| p.name.clone());
            let read_only = library.palettes.get(active).is_none_or(|p| p.read_only);

            let mut action = None;
            let combo = self.dropdown(ui, "palette_combo", "Palette", &current_name, 78.0);
            egui::Popup::menu(&combo).width(140.0).show(|ui| {
                for (i, p) in library.palettes.palettes().iter().enumerate() {
                    if ui.selectable_label(i == active, &p.name).clicked() {
                        action = Some(PaletteAction::Select(i));
                    }
                }
                ui.separator();
                if ui.button("New palette").clicked() {
                    action = Some(PaletteAction::New);
                }
                if !read_only {
                    if ui.button("Rename...").clicked() {
                        action = Some(PaletteAction::StartRename);
                    }
                    if ui.button("Delete palette").clicked() {
                        action = Some(PaletteAction::Delete);
                    }
                }
            });

            if let Some(name) = &mut self.renaming {
                let id = self.id.with("rename");
                let resp = ui.add(
                    TextEdit::singleline(name)
                        .id(id)
                        .desired_width(ui.available_width() - 4.0)
                        .hint_text("Palette name"),
                );
                if !resp.has_focus() && !resp.lost_focus() {
                    resp.request_focus();
                }
                if resp.lost_focus() {
                    let (enter, esc) =
                        ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    if enter {
                        action = Some(PaletteAction::Rename(name.clone()));
                    } else if !esc {
                        // Clicked away. Esc is left to `after_frame`, which cancels only the rename.
                        self.renaming = None;
                    }
                }
            } else {
                self.palette_swatches(ui, library, active, read_only, &mut action);
            }

            if let Some(a) = action {
                self.apply_palette_action(a, library);
            }
        });
    }

    fn palette_swatches(
        &mut self,
        ui: &mut Ui,
        library: &mut ColourLibrary,
        active: usize,
        read_only: bool,
        action: &mut Option<PaletteAction>,
    ) {
        let colours = library
            .palettes
            .get(active)
            .map(|p| p.colours.clone())
            .unwrap_or_default();
        self.roving[1] = self.roving[1].min(colours.len().saturating_sub(1));
        let plus_w = SWATCH + SWATCH_GAP;
        let area_w = (ui.available_width() - plus_w).max(SWATCH);
        let mut focused = None;
        let mut slots = Vec::with_capacity(colours.len());
        egui::ScrollArea::horizontal()
            .id_salt(self.id.with("palette_scroll"))
            .max_width(area_w)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SWATCH_GAP;
                    let drag = if read_only {
                        Sense::hover()
                    } else {
                        Sense::DRAG
                    };
                    for (i, c) in colours.iter().enumerate() {
                        let resp = self.swatch(ui, Row::Palette, i, *c, drag);
                        slots.push(resp.rect);
                        if resp.clicked() {
                            *action = Some(PaletteAction::Apply(*c));
                        }
                        if resp.has_focus() {
                            focused = Some(i);
                            resp.scroll_to_me(None);
                            let (del, enter, l, r, ctrl) = ui.input(|inp| {
                                (
                                    inp.key_pressed(Key::Delete),
                                    inp.key_pressed(Key::Enter),
                                    inp.key_pressed(Key::ArrowLeft),
                                    inp.key_pressed(Key::ArrowRight),
                                    inp.modifiers.ctrl || inp.modifiers.command,
                                )
                            });
                            if del && !read_only {
                                *action = Some(PaletteAction::Remove(i));
                            } else if enter {
                                *action = Some(PaletteAction::Apply(*c));
                            } else if ctrl && !read_only && (l || r) {
                                let to = if r { i + 1 } else { i.saturating_sub(1) };
                                *action = Some(PaletteAction::Move(i, to));
                            }
                        }
                        if resp.drag_started() {
                            self.palette_drag = Some(i);
                        }
                        if !read_only {
                            resp.context_menu(|ui| {
                                if ui.button("Remove").clicked() {
                                    *action = Some(PaletteAction::Remove(i));
                                }
                                if ui.button("Rename palette...").clicked() {
                                    *action = Some(PaletteAction::StartRename);
                                }
                            });
                        }
                    }
                });
            });
        // Drag to reorder: drop where the pointer is released.
        if let Some(from) = self.palette_drag
            && !ui.input(|i| i.pointer.any_down())
        {
            self.palette_drag = None;
            let to = ui
                .input(|i| i.pointer.interact_pos())
                .and_then(|p| nearest_slot(&slots, p));
            if let Some(to) = to
                && to != from
            {
                *action = Some(PaletteAction::Move(from, to));
            }
        }
        self.roving_keys(ui, Row::Palette, focused, colours.len());

        let (rect, _) = ui.allocate_exact_size(Vec2::splat(SWATCH + 2.0), Sense::hover());
        let plus = self.icon_button(ui, rect, "palette_add", Icon::Plus, false);
        let plus = plus.on_hover_text("Add the current colour to this palette");
        if plus.clicked() {
            *action = Some(PaletteAction::AddCurrent);
        }
    }

    fn apply_palette_action(&mut self, action: PaletteAction, library: &mut ColourLibrary) {
        let active = library.active_palette;
        let result = match action {
            PaletteAction::Select(i) => {
                library.active_palette = i;
                Ok(())
            }
            PaletteAction::Apply(c) => {
                self.set_hsv(Hsv::from_rgb8(c, self.hsv.h));
                self.commit(library);
                Ok(())
            }
            PaletteAction::AddCurrent => self.add_to_palette(self.colour(), library),
            PaletteAction::Remove(i) => library.palettes.remove_colour(active, i),
            PaletteAction::Move(from, to) => {
                self.roving[1] = to;
                library.palettes.move_colour(active, from, to)
            }
            PaletteAction::New => {
                let name = library.palettes.next_free_name();
                library.palettes.create(&name).map(|i| {
                    library.active_palette = i;
                })
            }
            PaletteAction::StartRename => {
                self.renaming = library.palettes.get(active).map(|p| p.name.clone());
                Ok(())
            }
            PaletteAction::Rename(name) => {
                self.renaming = None;
                library.palettes.rename(active, name.trim())
            }
            PaletteAction::Delete => library.palettes.delete(active),
        };
        self.message = result.err().map(|e| e.to_string());
    }

    fn add_to_palette(
        &mut self,
        c: Rgb8,
        library: &mut ColourLibrary,
    ) -> Result<(), crate::StudioError> {
        let editable = library
            .palettes
            .get(library.active_palette)
            .is_some_and(|p| !p.read_only);
        if !editable {
            let name = library.palettes.next_free_name();
            library.active_palette = library.palettes.create(&name)?;
        }
        library.palettes.add_colour(library.active_palette, c)
    }

    fn harmony_row(&mut self, ui: &mut Ui, library: &mut ColourLibrary) {
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            row_label(ui, "Harmony");
            let combo = self.dropdown(
                ui,
                "harmony_combo",
                "Harmony",
                library.harmony.label(),
                104.0,
            );
            egui::Popup::menu(&combo).width(140.0).show(|ui| {
                for rule in Harmony::ALL {
                    ui.selectable_value(&mut library.harmony, rule, rule.label());
                }
            });
            ui.spacing_mut().item_spacing.x = SWATCH_GAP;
            let chips = library.harmony.generate(self.colour());
            self.roving[2] = self.roving[2].min(chips.len().saturating_sub(1));
            let mut focused = None;
            for (i, c) in chips.iter().enumerate() {
                let resp = self.swatch(ui, Row::Harmony, i, *c, Sense::hover());
                let resp = resp.on_hover_text("Click applies; Shift+click adds it to the palette");
                if resp.clicked() {
                    if ui.input(|inp| inp.modifiers.shift) {
                        let r = self.add_to_palette(*c, library);
                        self.message = r.err().map(|e| e.to_string());
                    } else {
                        self.set_hsv(Hsv::from_rgb8(*c, self.hsv.h));
                        self.commit(library);
                    }
                }
                if resp.has_focus() {
                    focused = Some(i);
                }
            }
            self.roving_keys(ui, Row::Harmony, focused, chips.len());
        });
    }

    /// A dropdown button in the Studio style (the menu opens with `Popup::menu`).
    fn dropdown(&mut self, ui: &mut Ui, key: &str, name: &str, text: &str, width: f32) -> Response {
        let id = self.id.with(key);
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
        let resp = ui.interact(rect, id, Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let painter = ui.painter();
        let fill = if resp.hovered() || open {
            theme::HOVER
        } else {
            theme::DEEP
        };
        painter.rect_filled(rect, 4.0, fill);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );
        let galley = painter.layout(
            text.to_owned(),
            FontId::proportional(13.0),
            theme::TEXT,
            rect.width() - 26.0,
        );
        painter.galley(
            pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::TEXT,
        );
        // Chevron.
        let c = pos2(rect.right() - 11.0, rect.center().y);
        let chevron = Stroke::new(1.3, theme::TEXT_DIM);
        painter.line_segment([c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0)], chevron);
        painter.line_segment([c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)], chevron);
        if resp.has_focus() {
            focus_ring(painter, rect, 4.0);
        }
        let label = format!("{name}, {text}");
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(accesskit::Role::ComboBox);
            node.set_label(label);
        });
        resp
    }

    fn icon_button(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        key: &str,
        icon: Icon,
        active: bool,
    ) -> Response {
        let id = self.id.with(key);
        let resp = ui.interact(rect, id, Sense::click());
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, 4.0, theme::ACCENT.gamma_multiply(0.35));
        } else if resp.hovered() {
            painter.rect_filled(rect, 4.0, theme::HOVER);
        }
        let colour = if active || resp.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        paint_icon(painter, rect, icon, colour);
        if resp.has_focus() {
            focus_ring(painter, rect, 4.0);
        }
        let label = match icon {
            Icon::Pipette => "Eyedropper",
            Icon::Close => "Close colour popover",
            Icon::Plus => "Add colour to palette",
        };
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(accesskit::Role::Button);
            node.set_label(label);
            if icon == Icon::Pipette {
                node.set_toggled(if active {
                    accesskit::Toggled::True
                } else {
                    accesskit::Toggled::False
                });
            }
        });
        resp
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PaletteAction {
    Select(usize),
    Apply(Rgb8),
    AddCurrent,
    Remove(usize),
    Move(usize, usize),
    New,
    StartRename,
    Rename(String),
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Pipette,
    Close,
    Plus,
}

fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, colour: Color32) {
    let c = rect.center();
    let s = Stroke::new(1.5, colour);
    match icon {
        Icon::Close => {
            let d = 4.5;
            painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], s);
            painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], s);
        }
        Icon::Plus => {
            let d = 4.5;
            painter.line_segment([c + vec2(-d, 0.0), c + vec2(d, 0.0)], s);
            painter.line_segment([c + vec2(0.0, -d), c + vec2(0.0, d)], s);
        }
        Icon::Pipette => {
            // A dropper drawn from a shaft and a bulb.
            painter.line_segment(
                [c + vec2(-6.0, 6.0), c + vec2(1.5, -1.5)],
                Stroke::new(2.0, colour),
            );
            painter.circle_filled(c + vec2(3.5, -3.5), 3.0, colour);
            painter.line_segment([c + vec2(-1.0, -5.0), c + vec2(5.0, 1.0)], s);
        }
    }
}

/// The ring and square markers: filled circle, 3 px white border, 1 px graphite outer ring.
fn paint_marker(painter: &egui::Painter, at: Pos2, radius: f32, fill: Color32) {
    painter.circle_filled(at, radius + 1.0, theme::DEEP);
    painter.circle_filled(at, radius, Color32::WHITE);
    painter.circle_filled(at, radius - 3.0, fill);
}

fn focus_ring(painter: &egui::Painter, rect: Rect, radius: f32) {
    painter.rect_stroke(
        rect.expand(2.0),
        radius + 2.0,
        Stroke::new(2.0, theme::FOCUS),
        StrokeKind::Outside,
    );
}

fn row_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(LABEL_W, ROW), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );
}

/// Keeps the arrow keys for this widget instead of moving focus.
fn lock_arrows(ui: &Ui, id: Id) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

fn select_all(ui: &Ui, id: Id, text: &str) {
    if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::select_all_str(text)));
        state.store(ui.ctx(), id);
    }
}

fn to_color32(c: Rgb8) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// The hue under a ring offset from the centre: 0 at the top, increasing clockwise.
fn ring_hue(v: Vec2) -> f32 {
    v.y.atan2(v.x).to_degrees() + 90.0
}

/// The slot whose centre is nearest to `p`, by distance in both x and y, so dropping works in
/// a single row, a wrapped row or a grid. `None` when there are no slots.
pub fn nearest_slot(slots: &[Rect], p: Pos2) -> Option<usize> {
    slots
        .iter()
        .enumerate()
        .min_by(|a, b| {
            a.1.center()
                .distance_sq(p)
                .total_cmp(&b.1.center().distance_sq(p))
        })
        .map(|(i, _)| i)
}

/// -180 to 180 degrees.
fn signed_degrees(d: f32) -> f32 {
    let w = super::maths::wrap_degrees(d);
    if w > 180.0 { w - 360.0 } else { w }
}

/// The pixel of a window capture under `pos` (points), if inside.
pub fn pixel_at(image: &egui::ColorImage, pos: Pos2, pixels_per_point: f32) -> Option<Rgb8> {
    let x = (pos.x * pixels_per_point).floor();
    let y = (pos.y * pixels_per_point).floor();
    let [w, h] = image.size;
    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
        return None;
    }
    let p = image.pixels[y as usize * w + x as usize];
    Some(Rgb8::new(p.r(), p.g(), p.b()))
}
