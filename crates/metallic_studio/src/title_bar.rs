//! Studio's own title bar (research doc `docs/research/2026-10-30-ui-overhaul.md`, section 4.2 and
//! task S6): 40 px of `bg-deep` with the emblem and the game's name, a centred search field, a ghost
//! Settings button and the caption buttons (minimise, maximise or restore, close).
//!
//! # Windows
//!
//! The window has no system decorations, but it keeps the system's window behaviour, because a
//! subclassed window procedure ([`native`]) answers `WM_NCHITTEST` from the regions the bar
//! publishes each frame ([`HitRegions`], tested in [`hit_test`]):
//! - `HTCAPTION` over the empty parts of the bar: drag to move, drag to a screen edge to snap,
//!   double-click to maximise or restore, Alt+Space and right-click for the system menu;
//! - `HTMAXBUTTON` over the maximise button: hovering it opens the Windows 11 snap layouts;
//!   pressing and releasing it maximises or restores, as the system button would;
//! - the resize edges and corners around the window (not while maximised);
//! - `HTCLIENT` everywhere else, so egui gets the search field, Settings, minimise and close.
//!
//! winit keeps `WS_CAPTION`, `WS_SIZEBOX` and `WS_MAXIMIZEBOX` on undecorated windows (it only
//! hides the frame in `WM_NCCALCSIZE`), so snapping, the shadow and rounded corners stay native.
//! The snap layouts flyout also needs the window to have no non-client strip at all: winit's
//! undecorated window keeps a 1 px one at the top (its drop-shadow hack), and with it Windows 11
//! never offers snap layouts on `HTMAXBUTTON` (checked against a plain Win32 window on build
//! 26200). So the subclass answers `WM_NCCALCSIZE` itself while the window is restored.
//!
//! # macOS and Linux
//!
//! The native title bar stays ([`CUSTOM`] is false) and this bar is not drawn: no custom
//! decorations where window-manager behaviour cannot be matched as reliably.
//!
//! # Cost
//!
//! The bar keeps its text laid out ([`CachedText`]) and its regions in fixed arrays, so a frame
//! with nothing hovered allocates nothing here. A tooltip allocates while it shows.

use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Id, Key, Modifiers, Pos2, Rect, Response,
    Role, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2, ViewportCommand, WidgetInfo, pos2,
    vec2,
};

use crate::theme::{self, fonts, icons};
use crate::widgets::button::focus_ring;

/// True where Studio draws its own title bar and turns the system one off (Windows only).
pub const CUSTOM: bool = cfg!(windows);

/// Height of the bar in points (doc 3.3: title bar 40).
pub const HEIGHT: f32 = 40.0;
/// Width of a caption button (the Windows 11 size).
pub const CAPTION_WIDTH: f32 = 46.0;
/// Left padding before the emblem.
const PAD: f32 = 12.0;
/// The emblem's size.
const EMBLEM: f32 = 20.0;
/// Gap between the emblem and the title.
const EMBLEM_GAP: f32 = 8.0;
/// The search field: 28 high in the bar (ghost buttons are 28 here, doc 3.7).
const SEARCH_HEIGHT: f32 = 28.0;
/// The search field's widest and narrowest; narrower than this and it is not drawn.
const SEARCH_MAX: f32 = 440.0;
const SEARCH_MIN: f32 = 200.0;
/// Room kept between the search field and its neighbours.
const SEARCH_GAP: f32 = 16.0;
/// The ghost Settings button (icon only) and its gap to the caption buttons.
const GHOST: Vec2 = vec2(32.0, 28.0);
const GHOST_GAP: f32 = 8.0;
/// Caption glyphs are drawn, not set in a font: 10 px, like the system's.
const GLYPH: f32 = 10.0;

/// Text shown in the search field on the start screen (doc 4.2).
pub const SEARCH_PLACEHOLDER: &str = "Search templates and recent games";
/// Text shown in the search field in the editor.
pub const SEARCH_PLACEHOLDER_EDITOR: &str = "Search";
/// Its tooltip.
pub const SEARCH_UNAVAILABLE: &str =
    "Search is not built yet. It will open with the command palette.";

/// Where each part of the bar goes, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The whole bar.
    pub bar: Rect,
    /// The emblem.
    pub emblem: Rect,
    /// Left edge of the title text (vertically centred in the bar).
    pub title_x: f32,
    /// The search field; `None` when the window is too narrow for it.
    pub search: Option<Rect>,
    /// The Settings ghost button.
    pub settings: Rect,
    /// Minimise.
    pub minimise: Rect,
    /// Maximise or restore.
    pub maximise: Rect,
    /// Close.
    pub close: Rect,
}

/// Lays the bar out in `bar`, with a title `title_width` wide.
pub fn layout(bar: Rect, title_width: f32) -> Layout {
    let button = |right: f32| {
        Rect::from_min_max(
            pos2(right - CAPTION_WIDTH, bar.top()),
            pos2(right, bar.bottom()),
        )
    };
    let close = button(bar.right());
    let maximise = button(close.left());
    let minimise = button(maximise.left());
    let settings = Rect::from_center_size(
        pos2(minimise.left() - GHOST_GAP - GHOST.x / 2.0, bar.center().y),
        GHOST,
    );
    let emblem = Rect::from_center_size(
        pos2(bar.left() + PAD + EMBLEM / 2.0, bar.center().y),
        Vec2::splat(EMBLEM),
    );
    let title_x = emblem.right() + EMBLEM_GAP;

    // Centred on the window; shrinks to stay clear of the title and the buttons.
    let left_limit = title_x + title_width + SEARCH_GAP;
    let right_limit = settings.left() - SEARCH_GAP;
    let half = (SEARCH_MAX / 2.0)
        .min(bar.center().x - left_limit)
        .min(right_limit - bar.center().x);
    let search = (2.0 * half >= SEARCH_MIN)
        .then(|| Rect::from_center_size(bar.center(), vec2(2.0 * half, SEARCH_HEIGHT)));
    Layout {
        bar,
        emblem,
        title_x,
        search,
        settings,
        minimise,
        maximise,
        close,
    }
}

/// A rectangle in physical pixels of the window's client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PxRect {
    /// Left.
    pub left: i32,
    /// Top.
    pub top: i32,
    /// Right (exclusive).
    pub right: i32,
    /// Bottom (exclusive).
    pub bottom: i32,
}

impl PxRect {
    /// `rect` (points) at `pixels_per_point`, rounded outwards.
    pub fn from_points(rect: Rect, pixels_per_point: f32) -> PxRect {
        PxRect {
            left: (rect.left() * pixels_per_point).floor() as i32,
            top: (rect.top() * pixels_per_point).floor() as i32,
            right: (rect.right() * pixels_per_point).ceil() as i32,
            bottom: (rect.bottom() * pixels_per_point).ceil() as i32,
        }
    }

    /// True when `(x, y)` is inside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The most egui controls the bar has (search, Settings, minimise, close).
const MAX_CONTROLS: usize = 4;

/// What the window procedure needs to answer a hit test: the bar, the maximise button and the
/// egui controls on the bar, in client pixels. Written by the bar each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitRegions {
    /// The bar is drawn (false during the splash, and on other systems).
    pub enabled: bool,
    /// The bar.
    pub bar: PxRect,
    /// The maximise button.
    pub maximise: PxRect,
    /// Controls egui handles (they answer `HTCLIENT`).
    pub controls: [PxRect; MAX_CONTROLS],
    /// How many of `controls` are in use.
    pub control_count: usize,
}

/// What a point of the window is, for `WM_NCHITTEST`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Ordinary content: egui handles it.
    Client,
    /// The empty part of the title bar: move, snap, double-click, system menu.
    Caption,
    /// The maximise button: snap layouts on hover.
    Maximise,
    /// Resize edges and corners.
    Left,
    /// Right edge.
    Right,
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Top-left corner.
    TopLeft,
    /// Top-right corner.
    TopRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom-right corner.
    BottomRight,
}

/// Classifies `(x, y)` (client pixels) in a client area `size` wide and high, with resize edges
/// `border` pixels thick (none while `maximised`).
pub fn hit_test(
    regions: &HitRegions,
    (x, y): (i32, i32),
    size: (i32, i32),
    border: i32,
    maximised: bool,
) -> Hit {
    if !regions.enabled {
        return Hit::Client;
    }
    if !maximised {
        let (left, right) = (x < border, x >= size.0 - border);
        let (top, bottom) = (y < border, y >= size.1 - border);
        match (left, right, top, bottom) {
            (true, _, true, _) => return Hit::TopLeft,
            (_, true, true, _) => return Hit::TopRight,
            (true, _, _, true) => return Hit::BottomLeft,
            (_, true, _, true) => return Hit::BottomRight,
            (true, ..) => return Hit::Left,
            (_, true, ..) => return Hit::Right,
            (_, _, true, _) => return Hit::Top,
            (.., true) => return Hit::Bottom,
            _ => {}
        }
    }
    if !regions.bar.contains(x, y) {
        return Hit::Client;
    }
    if regions.maximise.contains(x, y) {
        return Hit::Maximise;
    }
    let controls = &regions.controls[..regions.control_count.min(MAX_CONTROLS)];
    if controls.iter().any(|r| r.contains(x, y)) {
        Hit::Client
    } else {
        Hit::Caption
    }
}

/// State shared between the bar (egui) and the window procedure (same thread, behind a mutex so
/// it can be shared at all).
#[derive(Debug, Default)]
pub struct Shared {
    state: Mutex<SharedState>,
}

#[derive(Debug, Default, Clone, Copy)]
struct SharedState {
    regions: HitRegions,
    /// The pointer is over the maximise button (non-client, so egui cannot see it).
    max_hovered: bool,
    /// The maximise button is held down.
    max_pressed: bool,
}

impl Shared {
    fn with<R>(&self, f: impl FnOnce(&mut SharedState) -> R) -> R {
        f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The regions the bar published last.
    pub fn regions(&self) -> HitRegions {
        self.with(|s| s.regions)
    }
}

/// A piece of text laid out once and kept until its text, the scale or the font atlas changes.
#[derive(Default)]
pub struct CachedText {
    galley: Option<Arc<Galley>>,
    text: String,
    pixels_per_point: f32,
    /// The font atlas fill when this was laid out: it only shrinks when egui clears the atlas,
    /// which also makes old layouts invalid.
    atlas_fill: f32,
}

impl CachedText {
    /// The layout of `text`; `font` is only called when a new layout is needed.
    pub fn get(&mut self, ui: &Ui, text: &str, font: impl FnOnce() -> FontId) -> Arc<Galley> {
        let ppp = ui.ctx().pixels_per_point();
        let fill = ui.ctx().fonts(|f| f.font_atlas_fill_ratio());
        let stale = self.galley.is_none()
            || self.text != text
            || self.pixels_per_point != ppp
            || fill < self.atlas_fill;
        if stale {
            self.text.clear();
            self.text.push_str(text);
            self.galley = Some(ui.painter().layout_no_wrap(
                text.to_owned(),
                font(),
                Color32::PLACEHOLDER,
            ));
            self.pixels_per_point = ppp;
        }
        self.atlas_fill = ui.ctx().fonts(|f| f.font_atlas_fill_ratio());
        match &self.galley {
            Some(galley) => Arc::clone(galley),
            None => unreachable!("laid out above"),
        }
    }

    /// True when the last [`Self::get`] reused the layout (for tests).
    pub fn is_laid_out(&self) -> bool {
        self.galley.is_some()
    }
}

/// What the bar is showing this frame.
pub struct TitleBarInput<'a> {
    /// The open game's name, or "Metallic Studio".
    pub title: &'a str,
    /// The emblem.
    pub icon: Option<&'a TextureHandle>,
    /// Ctrl+K belongs to the search field (on the start screen; the editor binds it to grid
    /// snapping).
    pub search_shortcut: bool,
    /// The start screen is showing (the search hint names what it will search).
    pub start_screen: bool,
}

/// What the user asked for through the bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleBarOutput {
    /// Open the Settings dialog.
    pub open_settings: bool,
}

/// The title bar. Keep one for the life of the window.
#[derive(Default)]
pub struct TitleBar {
    shared: Arc<Shared>,
    hooked: bool,
    title: CachedText,
    search: CachedText,
    search_icon: CachedText,
    keycap: CachedText,
    gear: CachedText,
}

impl TitleBar {
    /// A new bar (the window procedure is hooked on the first [`Self::attach`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// The regions shared with the window procedure.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Hooks the window procedure of `frame`'s window (Windows; once). Call before [`Self::show`].
    pub fn attach(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        if self.hooked {
            return;
        }
        self.hooked = true;
        #[cfg(windows)]
        {
            use wgpu::rwh::{HasWindowHandle, RawWindowHandle};
            match frame.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => {
                    if !native::install(handle.hwnd.get(), Arc::clone(&self.shared), ctx.clone()) {
                        log::warn!("title bar: could not hook the window; snapping may not work");
                    }
                }
                _ => log::warn!("title bar: no Win32 window handle"),
            }
        }
        #[cfg(not(windows))]
        let _ = (frame, ctx);
    }

    /// Tells the window procedure the bar is not drawn (the splash).
    pub fn hide(&self) {
        self.shared.with(|s| s.regions.enabled = false);
    }

    /// Draws the bar into `ui` (a 40 px top panel) and publishes its regions.
    pub fn show(&mut self, ui: &mut Ui, input: &TitleBarInput<'_>) -> TitleBarOutput {
        let mut out = TitleBarOutput::default();
        let ctx = ui.ctx().clone();
        let bar = ui.max_rect();
        let (focused, maximised) =
            ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().maximized));
        let maximised = maximised.unwrap_or(false);
        // An inactive window's bar dims, like the system's.
        let ink = if focused {
            theme::text_1()
        } else {
            theme::text_3()
        };
        let glyph_ink = if focused {
            theme::text_2()
        } else {
            theme::text_3()
        };

        let painter = ui.painter().clone();
        painter.rect_filled(bar, CornerRadius::ZERO, theme::bg_deep());
        painter.hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, theme::line()),
        );

        let title = self.title.get(ui, input.title, fonts::strong);
        let at = layout(bar, title.size().x);

        // Emblem and title.
        if let Some(icon) = input.icon {
            let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
            painter.image(icon.id(), at.emblem, uv, Color32::WHITE);
        }
        painter.galley(
            pos2(at.title_x, bar.center().y - title.size().y / 2.0),
            title,
            ink,
        );

        // Search.
        if let Some(rect) = at.search {
            let hint = if input.start_screen {
                SEARCH_PLACEHOLDER
            } else {
                SEARCH_PLACEHOLDER_EDITOR
            };
            self.search_field(ui, rect, input.search_shortcut, hint);
        }

        // Settings.
        let gear = ui.interact(
            at.settings,
            Id::unique("title-bar-settings"),
            Sense::click(),
        );
        gear.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Settings"));
        let fill = button_fill(&gear);
        if fill != Color32::TRANSPARENT {
            painter.rect_filled(at.settings, CornerRadius::same(theme::R_SM), fill);
        }
        let gear_glyph = self
            .gear
            .get(ui, icons::GEAR_SIX.encode_utf8(&mut [0; 4]), || {
                fonts::icon(16.0)
            });
        let gear_ink = if gear.hovered() {
            theme::text_1()
        } else {
            glyph_ink
        };
        painter.galley(
            at.settings.center() - gear_glyph.size() / 2.0,
            gear_glyph,
            gear_ink,
        );
        if gear.has_focus() {
            focus_ring(&painter, at.settings, CornerRadius::same(theme::R_SM));
        }
        if gear.clicked() {
            out.open_settings = true;
        }
        gear.on_hover_text("Settings");

        // Caption buttons.
        let min = ui.interact(
            at.minimise,
            Id::unique("title-bar-minimise"),
            Sense::click(),
        );
        min.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Minimise"));
        caption_fill(&painter, at.minimise, button_fill(&min));
        let ink_of = |r: &Response| {
            if r.hovered() {
                theme::text_1()
            } else {
                glyph_ink
            }
        };
        paint_minimise(&painter, at.minimise.center(), ink_of(&min));
        caption_focus(&painter, at.minimise, &min);
        if min.clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }

        let (max_hovered, max_pressed) = self.shared.with(|s| (s.max_hovered, s.max_pressed));
        let max = ui.interact(
            at.maximise,
            Id::unique("title-bar-maximise"),
            Sense::click(),
        );
        let max_name = if maximised { "Restore" } else { "Maximise" };
        max.widget_info(|| WidgetInfo::labeled(Role::Button, true, max_name));
        let max_fill = if max_pressed || max.is_pointer_button_down_on() {
            theme::bg_active()
        } else if max_hovered || max.hovered() {
            theme::bg_hover()
        } else {
            Color32::TRANSPARENT
        };
        caption_fill(&painter, at.maximise, max_fill);
        let max_ink = if max_hovered || max.hovered() {
            theme::text_1()
        } else {
            glyph_ink
        };
        if maximised {
            paint_restore(&painter, at.maximise.center(), max_ink);
        } else {
            paint_maximise(&painter, at.maximise.center(), max_ink);
        }
        caption_focus(&painter, at.maximise, &max);
        // On Windows the pointer never reaches this widget (the window procedure owns it); the
        // click arrives from the keyboard, or from the pointer on other systems.
        if max.clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximised));
        }

        let close = ui.interact(at.close, Id::unique("title-bar-close"), Sense::click());
        close.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Close"));
        let close_hot = close.hovered() || close.is_pointer_button_down_on();
        caption_fill(
            &painter,
            at.close,
            if close_hot {
                theme::caption_close()
            } else {
                Color32::TRANSPARENT
            },
        );
        let close_ink = if close_hot {
            theme::on_danger()
        } else {
            glyph_ink
        };
        paint_close(&painter, at.close.center(), close_ink);
        caption_focus(&painter, at.close, &close);
        if close.clicked() {
            // The same path as Alt+F4: the app's unsaved-changes guard sees `close_requested`.
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        self.publish(&ctx, &at);
        out
    }

    /// The search field: drawn disabled until the command palette exists, but focusable (Tab,
    /// and Ctrl+K on the start screen) so its tooltip can say why.
    fn search_field(&mut self, ui: &mut Ui, rect: Rect, shortcut: bool, hint: &str) {
        let id = Id::unique("title-bar-search");
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(Role::TextInput, false, "Search (not available yet)")
        });
        if shortcut && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            response.request_focus();
        }
        if response.clicked() {
            response.request_focus();
        }
        let painter = ui.painter();
        // The disabled input of doc 3.7: `bg-panel` with `text-off`.
        let radius = CornerRadius::same(theme::R_SM);
        painter.rect_filled(rect, radius, theme::bg_panel());
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::line()),
            StrokeKind::Inside,
        );
        let icon =
            self.search_icon
                .get(ui, icons::MAGNIFYING_GLASS.encode_utf8(&mut [0; 4]), || {
                    fonts::icon(16.0)
                });
        let icon_pos = pos2(rect.left() + 10.0, rect.center().y - icon.size().y / 2.0);
        let text_x = icon_pos.x + icon.size().x + 8.0;
        painter.galley(icon_pos, icon, theme::text_off());
        let keycap = shortcut.then(|| self.keycap.get(ui, "Ctrl K", fonts::caption));
        let keycap_width = keycap.as_ref().map_or(0.0, |g| keycap_size(g).x + 8.0);
        let text = self.search.get(ui, hint, fonts::body);
        // Keep the placeholder inside the field; drop it when it would not fit.
        if text_x + text.size().x <= rect.right() - 10.0 - keycap_width {
            painter.galley(
                pos2(text_x, rect.center().y - text.size().y / 2.0),
                text,
                theme::text_off(),
            );
        }
        if let Some(keycap) = keycap {
            let size = keycap_size(&keycap);
            let cap =
                Align2::RIGHT_CENTER.align_size_within_rect(size, rect.shrink2(vec2(6.0, 0.0)));
            painter.rect_filled(cap, CornerRadius::same(theme::R_XS), theme::bg_active());
            painter.galley(
                cap.center() - keycap.size() / 2.0,
                keycap,
                theme::text_off(),
            );
        }
        if response.has_focus() {
            focus_ring(ui.painter(), rect, radius);
            response.show_tooltip_text(SEARCH_UNAVAILABLE);
        }
        response.on_hover_text(SEARCH_UNAVAILABLE);
    }

    /// Writes this frame's regions for the window procedure, in client pixels.
    fn publish(&self, ctx: &egui::Context, at: &Layout) {
        let ppp = ctx.pixels_per_point();
        let px = |r: Rect| PxRect::from_points(r, ppp);
        let mut regions = HitRegions {
            enabled: true,
            bar: px(at.bar),
            maximise: px(at.maximise),
            ..HitRegions::default()
        };
        let controls = [
            at.search,
            Some(at.settings),
            Some(at.minimise),
            Some(at.close),
        ];
        for rect in controls.into_iter().flatten() {
            regions.controls[regions.control_count] = px(rect);
            regions.control_count += 1;
        }
        self.shared.with(|s| s.regions = regions);
    }
}

/// Ghost fill for a widget's state (doc 3.7: hover `bg-hover`, pressed `bg-active`).
fn button_fill(response: &Response) -> Color32 {
    if response.is_pointer_button_down_on() {
        theme::bg_active()
    } else if response.hovered() {
        theme::bg_hover()
    } else {
        Color32::TRANSPARENT
    }
}

fn caption_fill(painter: &egui::Painter, rect: Rect, fill: Color32) {
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
    }
}

/// Caption buttons touch the window edge, so their focus ring sits inside.
fn caption_focus(painter: &egui::Painter, rect: Rect, response: &Response) {
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(3.0),
            CornerRadius::same(theme::R_XS),
            Stroke::new(2.0, theme::focus()),
            StrokeKind::Inside,
        );
    }
}

/// Keycap size for a laid-out label (16 high, 5 side padding, at least 18 wide; doc 3.7).
fn keycap_size(galley: &Galley) -> Vec2 {
    vec2((galley.size().x + 10.0).max(18.0), 16.0)
}

/// Snaps a glyph's centre to the pixel grid so 1 px lines stay sharp.
fn crisp(painter: &egui::Painter, center: Pos2) -> Pos2 {
    let ppp = painter.ctx().pixels_per_point();
    let snap = |v: f32| ((v * ppp).round() + 0.5) / ppp;
    pos2(snap(center.x), snap(center.y))
}

/// A whole number of physical pixels (1 up to 200 %), so glyph lines stay sharp.
fn hairline(painter: &egui::Painter, ink: Color32) -> Stroke {
    let ppp = painter.ctx().pixels_per_point();
    Stroke::new(ppp.floor().max(1.0) / ppp, ink)
}

fn paint_minimise(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let c = crisp(painter, center);
    let h = GLYPH / 2.0;
    painter.hline(c.x - h..=c.x + h, c.y, hairline(painter, ink));
}

fn paint_maximise(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let c = crisp(painter, center);
    let rect = Rect::from_center_size(c, Vec2::splat(GLYPH));
    painter.rect_stroke(
        rect,
        CornerRadius::same(1),
        hairline(painter, ink),
        StrokeKind::Middle,
    );
}

fn paint_restore(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let c = crisp(painter, center);
    let stroke = hairline(painter, ink);
    let s = GLYPH - 2.0;
    // The front window, and the back one peeking out top-right.
    let front = Rect::from_min_size(
        pos2(c.x - GLYPH / 2.0, c.y - GLYPH / 2.0 + 2.0),
        Vec2::splat(s),
    );
    painter.rect_stroke(front, CornerRadius::same(1), stroke, StrokeKind::Middle);
    let (l, t, r, b) = (
        front.left() + 2.0,
        front.top() - 2.0,
        front.right() + 2.0,
        front.bottom() - 2.0,
    );
    painter.line_segment([pos2(l, front.top()), pos2(l, t)], stroke);
    painter.line_segment([pos2(l, t), pos2(r, t)], stroke);
    painter.line_segment([pos2(r, t), pos2(r, b)], stroke);
    painter.line_segment([pos2(r, b), pos2(front.right(), b)], stroke);
}

fn paint_close(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let c = crisp(painter, center);
    let h = GLYPH / 2.0;
    let stroke = hairline(painter, ink);
    painter.line_segment([pos2(c.x - h, c.y - h), pos2(c.x + h, c.y + h)], stroke);
    painter.line_segment([pos2(c.x + h, c.y - h), pos2(c.x - h, c.y + h)], stroke);
}

/// The Windows side: a window subclass that answers hit tests from [`Shared`] and runs the
/// maximise button.
#[cfg(windows)]
mod native {
    use std::sync::Arc;

    use eframe::egui;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
        HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, PostMessageW, SC_MAXIMIZE,
        SC_RESTORE, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_NCCALCSIZE, WM_NCDESTROY,
        WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_NCMOUSEMOVE, WM_SYSCOMMAND,
    };

    use super::{Hit, Shared, hit_test};

    /// "MTLB": this subclass's id on the window.
    const SUBCLASS_ID: usize = 0x4d54_4c42;

    struct Hook {
        shared: Arc<Shared>,
        ctx: egui::Context,
    }

    /// Subclasses `hwnd` (a window of this thread). False if Windows refused.
    pub fn install(hwnd: isize, shared: Arc<Shared>, ctx: egui::Context) -> bool {
        let hwnd = hwnd as HWND;
        let data = Box::into_raw(Box::new(Hook { shared, ctx }));
        // SAFETY: `hwnd` is this thread's live window (from eframe); `data` stays valid until
        // WM_NCDESTROY, where the subclass is removed and the box dropped.
        let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, data as usize) };
        if ok == 0 {
            // SAFETY: Windows did not keep the pointer.
            drop(unsafe { Box::from_raw(data) });
            return false;
        }
        // Recalculate the frame now, so WM_NCCALCSIZE below drops winit's top strip at once.
        // SAFETY: plain Win32 call on our own window; no move, size or z-order change.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        true
    }

    fn code(hit: Hit) -> LRESULT {
        (match hit {
            Hit::Client => HTCLIENT,
            Hit::Caption => HTCAPTION,
            Hit::Maximise => HTMAXBUTTON,
            Hit::Left => HTLEFT,
            Hit::Right => HTRIGHT,
            Hit::Top => HTTOP,
            Hit::Bottom => HTBOTTOM,
            Hit::TopLeft => HTTOPLEFT,
            Hit::TopRight => HTTOPRIGHT,
            Hit::BottomLeft => HTBOTTOMLEFT,
            Hit::BottomRight => HTBOTTOMRIGHT,
        }) as LRESULT
    }

    /// Screen coordinates packed in an `LPARAM` (signed: monitors left of or above the primary).
    fn point(lparam: LPARAM) -> POINT {
        POINT {
            x: i32::from((lparam & 0xffff) as u16 as i16),
            y: i32::from(((lparam >> 16) & 0xffff) as u16 as i16),
        }
    }

    unsafe extern "system" fn subclass(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: `data` is the `Hook` boxed in `install`, alive until WM_NCDESTROY below.
        let hook = unsafe { &*(data as *const Hook) };
        let on_max = wparam == HTMAXBUTTON as WPARAM;
        match msg {
            WM_NCHITTEST => {
                let mut p = point(lparam);
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                // SAFETY: plain Win32 queries on our own window.
                let (border, maximised) = unsafe {
                    ScreenToClient(hwnd, &mut p);
                    GetClientRect(hwnd, &mut client);
                    let dpi = GetDpiForWindow(hwnd);
                    let border = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    (border, IsZoomed(hwnd) != 0)
                };
                let regions = hook.shared.regions();
                let hit = hit_test(
                    &regions,
                    (p.x, p.y),
                    (client.right, client.bottom),
                    border,
                    maximised,
                );
                return code(hit);
            }
            // winit's undecorated window keeps a 1 px non-client strip at the top (its drop-shadow
            // hack). Any non-client strip stops Windows 11 offering snap layouts on HTMAXBUTTON, so
            // the restored window's client area covers the whole window instead. Maximised, winit
            // already fits it to the work area without the strip.
            // SAFETY: plain query on our own window.
            WM_NCCALCSIZE if wparam != 0 && unsafe { IsZoomed(hwnd) } == 0 => {
                return 0;
            }
            WM_NCMOUSEMOVE => {
                let changed = hook.shared.with(|s| {
                    let was = s.max_hovered;
                    s.max_hovered = on_max;
                    if !on_max {
                        s.max_pressed = false;
                    }
                    was != on_max
                });
                if changed {
                    // Ask for WM_NCMOUSELEAVE, so the hover ends when the pointer goes.
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: a valid struct for our own window.
                    unsafe { TrackMouseEvent(&mut track) };
                    hook.ctx.request_repaint();
                }
            }
            WM_NCMOUSELEAVE => {
                hook.shared.with(|s| {
                    s.max_hovered = false;
                    s.max_pressed = false;
                });
                hook.ctx.request_repaint();
            }
            // The system would draw its own button and run a modal loop: the press is ours.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_max => {
                hook.shared.with(|s| s.max_pressed = true);
                hook.ctx.request_repaint();
                return 0;
            }
            WM_NCLBUTTONUP if on_max => {
                let pressed = hook.shared.with(|s| std::mem::take(&mut s.max_pressed));
                if pressed {
                    // SAFETY: plain Win32 calls on our own window.
                    unsafe {
                        let command = if IsZoomed(hwnd) != 0 {
                            SC_RESTORE
                        } else {
                            SC_MAXIMIZE
                        };
                        PostMessageW(hwnd, WM_SYSCOMMAND, command as WPARAM, 0);
                    }
                }
                hook.ctx.request_repaint();
                return 0;
            }
            WM_NCDESTROY => {
                // SAFETY: the window is going; nothing calls this subclass after removal.
                unsafe {
                    RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                    drop(Box::from_raw(data as *mut Hook));
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
            }
            _ => {}
        }
        // SAFETY: forwards to the next window procedure (winit's).
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(width: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(width, HEIGHT))
    }

    #[test]
    fn caption_buttons_sit_right_and_the_search_centres() {
        let at = layout(bar(1440.0), 110.0);
        assert_eq!(at.close.right(), 1440.0);
        assert_eq!(at.close.width(), CAPTION_WIDTH);
        assert_eq!(at.maximise.right(), at.close.left());
        assert_eq!(at.minimise.right(), at.maximise.left());
        assert_eq!(at.close.height(), HEIGHT);
        let search = at.search.expect("room for the search");
        assert_eq!(search.center().x, 720.0);
        assert_eq!(search.width(), SEARCH_MAX);
        assert_eq!(search.height(), SEARCH_HEIGHT);
        assert!(at.settings.right() < at.minimise.left());
        assert_eq!(at.emblem.size(), Vec2::splat(EMBLEM));
    }

    #[test]
    fn the_search_shrinks_then_hides_instead_of_covering_the_title() {
        let at = layout(bar(800.0), 100.0);
        let search = at.search.expect("still fits");
        assert!(search.width() < SEARCH_MAX, "shrunk");
        assert_eq!(search.center().x, 400.0, "still centred");
        assert!(search.left() >= at.title_x + 100.0 + SEARCH_GAP - 0.01);
        assert!(search.right() <= at.settings.left() - SEARCH_GAP + 0.01);
        // A long title or a narrow window: no search rather than an overlap.
        assert!(layout(bar(800.0), 300.0).search.is_none());
        assert!(layout(bar(560.0), 100.0).search.is_none());
    }

    fn regions() -> HitRegions {
        let at = layout(bar(1000.0), 100.0);
        let px = |r: Rect| PxRect::from_points(r, 1.5);
        let mut regions = HitRegions {
            enabled: true,
            bar: px(at.bar),
            maximise: px(at.maximise),
            ..HitRegions::default()
        };
        for r in [at.search.unwrap(), at.settings, at.minimise, at.close] {
            regions.controls[regions.control_count] = px(r);
            regions.control_count += 1;
        }
        regions
    }

    #[test]
    fn hit_test_finds_caption_maximise_controls_and_edges() {
        let r = regions();
        let size = (1500, 900); // 1000 x 600 points at 150 %
        let at = layout(bar(1000.0), 100.0);
        let px = |p: Pos2| ((p.x * 1.5) as i32, (p.y * 1.5) as i32);
        let hit = |p: (i32, i32)| hit_test(&r, p, size, 8, false);
        // Empty bar between the title and the search: the system moves the window.
        assert_eq!(hit(px(pos2(250.0, 20.0))), Hit::Caption);
        // The maximise button answers HTMAXBUTTON: Windows 11 shows snap layouts there.
        assert_eq!(hit(px(at.maximise.center())), Hit::Maximise);
        // egui controls stay client.
        for c in [
            at.minimise.center(),
            at.close.center(),
            at.settings.center(),
        ] {
            assert_eq!(hit(px(c)), Hit::Client);
        }
        assert_eq!(hit(px(at.search.unwrap().center())), Hit::Client);
        // Below the bar: content.
        assert_eq!(hit((700, 400)), Hit::Client);
        // Edges and corners resize.
        assert_eq!(hit((2, 400)), Hit::Left);
        assert_eq!(hit((1497, 400)), Hit::Right);
        assert_eq!(hit((700, 898)), Hit::Bottom);
        assert_eq!(hit((700, 3)), Hit::Top);
        assert_eq!(hit((1, 1)), Hit::TopLeft);
        assert_eq!(hit((1499, 0)), Hit::TopRight);
        assert_eq!(hit((0, 899)), Hit::BottomLeft);
        assert_eq!(hit((1499, 899)), Hit::BottomRight);
        // Maximised: no resize edges; the top row of the maximise button still snaps.
        let max_top = (px(at.maximise.center()).0, 1);
        assert_eq!(hit_test(&r, max_top, size, 8, true), Hit::Maximise);
        assert_eq!(hit_test(&r, (2, 400), size, 8, true), Hit::Client);
        // Not drawn (splash): everything is client.
        let off = HitRegions::default();
        assert_eq!(hit_test(&off, (700, 3), size, 8, false), Hit::Client);
    }

    #[test]
    fn pixel_rects_round_outwards() {
        let p = PxRect::from_points(Rect::from_min_max(pos2(-1.0, 0.0), pos2(1.0, 1.0)), 2.0);
        assert!(p.contains(-2, 0));
        assert!(!p.contains(2, 0));
    }
}
