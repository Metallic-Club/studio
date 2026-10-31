//! The Player's touch overlay (docs/design/input-actions.md sections 7 and 10.3): the automatic
//! thumbstick, look pad and buttons, real touch screens, and device emulation
//! (`--emulate phone|tablet|small-phone[-portrait]`) where the mouse acts as a finger.
//!
//! The layout, finger routing and drawing are `metallic_runtime::input::touch`; this module
//! connects them to winit, converts pixels to dp, fades the overlay in and out, loads
//! `TouchImage` files and measures what drawing costs.
use metallic_runtime::input::touch::{
    DeviceProfile, OverlayStyle, TouchButtonInfo, TouchControls, TouchPhase, paint,
};
use metallic_runtime::input::{DeviceKind, FingerEvent};
use metallic_runtime::{Game, InputRuntime};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

/// Fade in and out, seconds (`--t-base`, 140 ms); 0 under Reduce motion.
pub const FADE_SECONDS: f32 = 0.14;
/// dp per pixel on a desktop touch screen is `1 / (0.6 * scale_factor)`: 96 dpi at 100% (spec 4.3).
const DESKTOP_PX_PER_DP: f32 = 0.6;
/// The finger the left mouse button plays in emulation, and the pinned second finger.
const MOUSE_FINGER: u64 = 1000;
const PINNED_FINGER: u64 = 1001;
/// Draw timings kept for the end-of-run report (no allocation while recording).
const TIMING_CAPACITY: usize = 1 << 16;
/// The emulated device's window: at most this many pixels per dp, wide and tall.
const EMULATION_PX_PER_DP: f64 = 2.0;
const EMULATION_MAX_WINDOW: [f64; 2] = [1600.0, 1000.0];

/// Parses `--emulate`: `phone`, `tablet` or `small-phone`, optionally `-portrait`.
pub fn parse_emulation(name: &str) -> Option<DeviceProfile> {
    let (base, portrait) = match name.strip_suffix("-portrait") {
        Some(b) => (b, true),
        None => (name.strip_suffix("-landscape").unwrap_or(name), false),
    };
    let p = match base {
        "phone" => DeviceProfile::phone(),
        "tablet" => DeviceProfile::tablet(),
        "small-phone" => DeviceProfile::small_phone(),
        _ => return None,
    };
    Some(if portrait { p.portrait() } else { p })
}

/// The menu button moves the GUI down only when its top edge is in this top part of the screen.
const MENU_TOP_BAND: f32 = 0.25;

/// The GUI inset in points (#333) while the touch controls fully show: the ScreenGuis start below
/// the menu button, with the usual 8 dp gap, so a top-centre game banner does not cover it. 0 when
/// there is no menu button or a custom layout put it lower than the top band (spec 7.3).
pub fn gui_inset(controls: &TouchControls, view: &Viewport, scale: f32) -> f32 {
    let screen_h = controls.profile().size[1];
    let Some(menu) = controls
        .layout()
        .menu
        .filter(|m| m.center[1] - m.radius <= MENU_TOP_BAND * screen_h)
    else {
        return 0.0;
    };
    let bottom_dp = menu.center[1] + menu.radius + 8.0;
    let scale = if scale > 0.0 { scale } else { 1.0 };
    (view.origin[1] + bottom_dp * view.px_per_dp) / scale
}

/// The window size for an emulated device, in physical pixels.
pub fn emulation_window(profile: &DeviceProfile) -> (u32, u32) {
    let [w, h] = profile.size.map(f64::from);
    let k = EMULATION_PX_PER_DP
        .min(EMULATION_MAX_WINDOW[0] / w)
        .min(EMULATION_MAX_WINDOW[1] / h);
    ((w * k).round() as u32, (h * k).round() as u32)
}

/// Where the dp screen sits in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Physical pixels per dp.
    pub px_per_dp: f32,
    /// Window pixel of dp (0, 0).
    pub origin: [f32; 2],
    /// The screen in dp.
    pub size: [f32; 2],
}

impl Viewport {
    /// The dp position of a window pixel.
    pub fn to_dp(self, px: [f32; 2]) -> [f32; 2] {
        [
            (px[0] - self.origin[0]) / self.px_per_dp,
            (px[1] - self.origin[1]) / self.px_per_dp,
        ]
    }
}

/// The viewport for a window of `size` pixels at `scale` (the OS scale factor). An emulated
/// device keeps its dp size and is letterboxed into the window.
pub fn viewport(emulated: Option<&DeviceProfile>, size: (u32, u32), scale: f32) -> Viewport {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    match emulated {
        Some(p) => {
            let k = (w / p.size[0]).min(h / p.size[1]).max(1e-3);
            Viewport {
                px_per_dp: k,
                origin: [(w - p.size[0] * k) / 2.0, (h - p.size[1] * k) / 2.0],
                size: p.size,
            }
        }
        None => {
            let k = (DESKTOP_PX_PER_DP * if scale > 0.0 { scale } else { 1.0 }).max(1e-3);
            Viewport {
                px_per_dp: k,
                origin: [0.0; 2],
                size: [w / k, h / k],
            }
        }
    }
}

/// The mouse as fingers (spec 10.2): the left button is one finger, a right click pins a second
/// finger at that spot or lifts it again.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct MouseFingers {
    left_down: bool,
    pinned: Option<[f32; 2]>,
}

impl MouseFingers {
    /// The left button went down or up at `at` dp.
    pub fn left(&mut self, down: bool, at: [f32; 2]) -> Option<FingerEvent> {
        if down == self.left_down {
            return None;
        }
        self.left_down = down;
        let phase = if down {
            TouchPhase::Down
        } else {
            TouchPhase::Up
        };
        Some(FingerEvent {
            finger: MOUSE_FINGER,
            phase,
            pos: at,
        })
    }

    /// The cursor moved to `at` dp.
    pub fn moved(&self, at: [f32; 2]) -> Option<FingerEvent> {
        self.left_down.then_some(FingerEvent {
            finger: MOUSE_FINGER,
            phase: TouchPhase::Move,
            pos: at,
        })
    }

    /// A right click at `at` dp: pins a finger there, or lifts the pinned one.
    pub fn right_click(&mut self, at: [f32; 2]) -> FingerEvent {
        match self.pinned.take() {
            Some(pin) => FingerEvent {
                finger: PINNED_FINGER,
                phase: TouchPhase::Up,
                pos: pin,
            },
            None => {
                self.pinned = Some(at);
                FingerEvent {
                    finger: PINNED_FINGER,
                    phase: TouchPhase::Down,
                    pos: at,
                }
            }
        }
    }

    /// Where the pinned finger is.
    pub fn pinned(&self) -> Option<[f32; 2]> {
        self.pinned
    }

    /// Forgets both fingers (focus loss).
    pub fn reset(&mut self) {
        *self = MouseFingers::default();
    }
}

/// Moves `alpha` toward `target` (0 or 1) by `dt` of a `fade` second fade; instant without one.
pub fn fade(alpha: f32, target: f32, dt: f32, fade: f32) -> f32 {
    if fade <= 0.0 || !dt.is_finite() {
        return target;
    }
    let step = (dt / fade).max(0.0);
    if alpha < target {
        (alpha + step).min(target)
    } else {
        (alpha - step).max(target)
    }
}

/// A relative path inside the project, or `None` (absolute paths, `..`, URLs).
fn project_file(project: &Path, image: &str) -> Option<PathBuf> {
    if image.is_empty() || image.contains("://") {
        return None;
    }
    let rel = Path::new(image);
    rel.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then(|| project.join(rel))
}

fn load_png(path: &Path) -> Result<egui::ColorImage, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("image too large")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w > 1024 || h > 1024 {
        return Err(format!("{w}x{h} is larger than 1024x1024"));
    }
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colour was not expanded".into()),
    };
    Ok(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

/// Overlay draw times for the run, in seconds.
#[derive(Debug)]
struct Timing {
    samples: Vec<f32>,
}

impl Timing {
    fn new() -> Self {
        Timing {
            samples: Vec::with_capacity(TIMING_CAPACITY),
        }
    }

    fn record(&mut self, seconds: f32) {
        if self.samples.len() < self.samples.capacity() {
            self.samples.push(seconds);
        }
    }

    fn report(&self) -> Option<String> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(f32::total_cmp);
        let ms = |s: f32| f64::from(s) * 1000.0;
        let mean = sorted.iter().map(|&s| f64::from(s)).sum::<f64>() / sorted.len() as f64;
        let pct = |p: f64| ms(sorted[((sorted.len() - 1) as f64 * p).round() as usize]);
        Some(format!(
            "metallic-player touch overlay draw: {} frames, mean {:.4} ms, p50 {:.4} ms, p99 {:.4} ms, max {:.4} ms",
            sorted.len(),
            mean * 1000.0,
            pct(0.5),
            pct(0.99),
            pct(1.0)
        ))
    }
}

/// The overlay: touch controls, emulation, fade and images.
pub struct TouchOverlay {
    controls: TouchControls,
    emulated: Option<DeviceProfile>,
    view: Viewport,
    mouse: MouseFingers,
    cursor: [f32; 2],
    alpha: f32,
    /// The player's opacity setting.
    opacity: f32,
    reduce_motion: bool,
    project: PathBuf,
    images: HashMap<String, Option<egui::TextureHandle>>,
    infos: Vec<TouchButtonInfo>,
    published: Option<(u64, bool, Viewport)>,
    timing: Timing,
    measure: bool,
}

impl TouchOverlay {
    /// An overlay for the project at `project`; `emulated` is the `--emulate` device.
    pub fn new(project: &Path, emulated: Option<DeviceProfile>, reduce_motion: bool) -> Self {
        let profile = emulated.unwrap_or_else(DeviceProfile::phone);
        TouchOverlay {
            controls: TouchControls::new(profile),
            emulated,
            view: viewport(emulated.as_ref(), (1, 1), 1.0),
            mouse: MouseFingers::default(),
            cursor: [0.0; 2],
            alpha: if emulated.is_some() { 1.0 } else { 0.0 },
            opacity: 1.0,
            reduce_motion,
            project: project.to_path_buf(),
            images: HashMap::new(),
            infos: Vec::new(),
            published: None,
            timing: Timing::new(),
            measure: emulated.is_some(),
        }
    }

    /// True in device emulation (the mouse is a finger).
    pub fn emulating(&self) -> bool {
        self.emulated.is_some()
    }

    /// Per frame, before the input is evaluated: follows the window size and the runtime's touch
    /// actions, fades, and tells the scripts where the buttons are.
    pub fn update(
        &mut self,
        rt: &mut InputRuntime,
        game: &mut Game,
        window: (u32, u32),
        scale: f32,
        time: f64,
        dt: f32,
    ) {
        self.view = viewport(self.emulated.as_ref(), window, scale);
        // The player's settings from the Controls page (spec 7.4).
        let settings = rt.overrides().touch;
        self.opacity = settings.opacity;
        let profile = self.emulated.unwrap_or_else(|| {
            let p = *self.controls.profile();
            DeviceProfile {
                size: self.view.size,
                insets: [0.0; 4],
                tablet: self.view.size[0].min(self.view.size[1]) >= 600.0,
                ..p
            }
        });
        self.controls.set_profile(
            profile
                .with_stick_mode(settings.stick_mode)
                .with_scale(settings.size)
                .with_large(settings.large_controls),
        );
        self.controls.set_px_per_dp(self.view.px_per_dp);
        self.controls.sync(rt, time);
        let shown = self.emulated.is_some() || rt.last_device() == DeviceKind::Touch;
        let fade_time = if self.reduce_motion {
            0.0
        } else {
            FADE_SECONDS
        };
        self.alpha = fade(self.alpha, f32::from(u8::from(shown)), dt, fade_time);
        let key = (rt.touch_generation(), shown, self.view);
        if self.published != Some(key) {
            self.published = Some(key);
            self.controls.button_infos(shown, &mut self.infos);
            game.set_touch_buttons(&self.infos);
        }
        // The GUI eases down and up with the controls' fade, not in one jump.
        game.set_gui_inset(gui_inset(&self.controls, &self.view, scale) * self.alpha);
    }

    /// A finger event in dp (input scripts, emulation).
    pub fn finger(&mut self, rt: &mut InputRuntime, e: FingerEvent, time: f64) {
        self.controls.touch(rt, e.finger, e.phase, e.pos, time);
    }

    /// A real touch at window pixel `px`.
    pub fn touch(
        &mut self,
        rt: &mut InputRuntime,
        id: u64,
        phase: winit::event::TouchPhase,
        px: [f32; 2],
        time: f64,
    ) {
        use winit::event::TouchPhase as W;
        let phase = match phase {
            W::Started => TouchPhase::Down,
            W::Moved => TouchPhase::Move,
            W::Ended => TouchPhase::Up,
            W::Cancelled => TouchPhase::Cancel,
        };
        let at = self.view.to_dp(px);
        self.controls.touch(rt, id, phase, at, time);
    }

    /// In emulation the mouse is a finger: returns true when the event was used.
    pub fn mouse_button(
        &mut self,
        rt: &mut InputRuntime,
        button: winit::event::MouseButton,
        down: bool,
        time: f64,
    ) -> bool {
        if !self.emulating() {
            return false;
        }
        let at = self.view.to_dp(self.cursor);
        let e = match button {
            winit::event::MouseButton::Left => self.mouse.left(down, at),
            winit::event::MouseButton::Right if down => Some(self.mouse.right_click(at)),
            _ => None,
        };
        if let Some(e) = e {
            self.finger(rt, e, time);
        }
        true
    }

    /// The cursor moved to window pixel `px`; returns true in emulation (the event was used).
    pub fn cursor_moved(&mut self, rt: &mut InputRuntime, px: [f32; 2], time: f64) -> bool {
        self.cursor = px;
        if !self.emulating() {
            return false;
        }
        if let Some(e) = self.mouse.moved(self.view.to_dp(px)) {
            self.finger(rt, e, time);
        }
        true
    }

    /// Lifts every finger (focus loss).
    pub fn release_all(&mut self, rt: &mut InputRuntime, time: f64) {
        self.mouse.reset();
        self.controls.release_all(rt, time);
    }

    /// Draws the overlay into an egui pass whose points are `scale` pixels.
    pub fn paint(&mut self, ctx: &egui::Context, scale: f32) {
        if self.alpha <= 0.0 && self.emulated.is_none() {
            return;
        }
        let started = Instant::now();
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("metallic_touch_overlay"),
        ));
        if self.emulated.is_some() {
            self.paint_letterbox(&painter, ctx.content_rect(), scale);
        }
        self.load_images(ctx);
        let style = OverlayStyle {
            origin: [self.view.origin[0] / scale, self.view.origin[1] / scale],
            points_per_dp: self.view.px_per_dp / scale,
            alpha: self.alpha,
            opacity: self.opacity,
            show_safe_area: self.emulated.is_some(),
        };
        let images = &self.images;
        let set = self.controls.set();
        let image = |slot: usize| {
            let def = set.button_in_slot(slot)?;
            images.get(&def.image)?.as_ref().map(|t| t.id())
        };
        paint(&painter, &self.controls, &style, &image);
        if let Some(pin) = self.mouse.pinned() {
            let p = egui::pos2(
                style.origin[0] + pin[0] * style.points_per_dp,
                style.origin[1] + pin[1] * style.points_per_dp,
            );
            let silver = egui::Color32::from_rgb(0xc3, 0xc0, 0xca);
            painter.circle_stroke(
                p,
                10.0 * style.points_per_dp,
                egui::Stroke::new(1.5, silver),
            );
            painter.circle_filled(p, 3.0 * style.points_per_dp, silver);
        }
        if self.measure {
            self.timing.record(started.elapsed().as_secs_f32());
        }
    }

    /// Bars around the emulated screen when the window has another shape.
    fn paint_letterbox(&self, painter: &egui::Painter, screen: egui::Rect, scale: f32) {
        let v = self.view;
        let min = egui::pos2(v.origin[0] / scale, v.origin[1] / scale);
        let max = egui::pos2(
            (v.origin[0] + v.size[0] * v.px_per_dp) / scale,
            (v.origin[1] + v.size[1] * v.px_per_dp) / scale,
        );
        let bar = egui::Color32::from_rgb(0x0c, 0x0b, 0x0f);
        for r in [
            egui::Rect::from_min_max(screen.min, egui::pos2(screen.max.x, min.y)),
            egui::Rect::from_min_max(egui::pos2(screen.min.x, max.y), screen.max),
            egui::Rect::from_min_max(egui::pos2(screen.min.x, min.y), egui::pos2(min.x, max.y)),
            egui::Rect::from_min_max(egui::pos2(max.x, min.y), egui::pos2(screen.max.x, max.y)),
        ] {
            if r.width() > 0.5 && r.height() > 0.5 {
                painter.rect_filled(r, 0.0, bar);
            }
        }
    }

    /// Loads `TouchImage` files the first time they are seen.
    fn load_images(&mut self, ctx: &egui::Context) {
        for b in &self.controls.set().buttons {
            if b.image.is_empty() || self.images.contains_key(&b.image) {
                continue;
            }
            let loaded = match project_file(&self.project, &b.image) {
                Some(path) => match load_png(&path) {
                    Ok(img) => Some(ctx.load_texture(&b.image, img, egui::TextureOptions::LINEAR)),
                    Err(e) => {
                        eprintln!("warning: touch image {}: {e}", b.image);
                        None
                    }
                },
                None => {
                    eprintln!(
                        "warning: touch image {}: not a PNG path inside the project",
                        b.image
                    );
                    None
                }
            };
            self.images.insert(b.image.clone(), loaded);
        }
    }

    /// The draw-time report for the end of the run.
    pub fn timing_report(&self) -> Option<String> {
        self.timing.report()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emulation_names() {
        assert_eq!(parse_emulation("phone"), Some(DeviceProfile::phone()));
        assert_eq!(
            parse_emulation("tablet-landscape"),
            Some(DeviceProfile::tablet())
        );
        let p = parse_emulation("small-phone-portrait").unwrap();
        assert_eq!(p.size, [360.0, 640.0]);
        assert_eq!(parse_emulation("watch"), None);
    }

    #[test]
    fn emulated_windows_fit_and_keep_the_shape() {
        assert_eq!(emulation_window(&DeviceProfile::phone()), (1600, 739));
        let (w, h) = emulation_window(&DeviceProfile::tablet());
        assert!(w <= 1600 && h <= 1000);
        assert!(((w as f32 / h as f32) - 1180.0 / 820.0).abs() < 0.01);
    }

    #[test]
    fn viewport_letterboxes_the_emulated_screen() {
        let phone = DeviceProfile::phone();
        let v = viewport(Some(&phone), (1688, 1000), 1.0);
        assert_eq!(v.px_per_dp, 2.0);
        assert_eq!(v.origin, [0.0, 110.0]);
        assert_eq!(v.to_dp([844.0, 500.0]), [422.0, 195.0]);
        // A desktop touch screen: 96 dpi at 100%.
        let d = viewport(None, (1920, 1080), 1.5);
        assert!((d.px_per_dp - 0.9).abs() < 1e-6);
        assert!((d.size[0] - 1920.0 / 0.9).abs() < 1e-3);
    }

    #[test]
    fn the_mouse_is_one_finger_and_a_right_click_pins_another() {
        let mut m = MouseFingers::default();
        assert_eq!(
            m.moved([1.0, 1.0]),
            None,
            "no finger while the button is up"
        );
        let down = m.left(true, [10.0, 20.0]).unwrap();
        assert_eq!((down.finger, down.phase), (MOUSE_FINGER, TouchPhase::Down));
        assert_eq!(m.left(true, [10.0, 20.0]), None, "repeats are ignored");
        assert_eq!(m.moved([12.0, 20.0]).unwrap().phase, TouchPhase::Move);
        assert_eq!(m.left(false, [12.0, 20.0]).unwrap().phase, TouchPhase::Up);
        let pin = m.right_click([300.0, 200.0]);
        assert_eq!((pin.finger, pin.phase), (PINNED_FINGER, TouchPhase::Down));
        assert_eq!(m.pinned(), Some([300.0, 200.0]));
        let lift = m.right_click([5.0, 5.0]);
        assert_eq!(
            (lift.phase, lift.pos),
            (TouchPhase::Up, [300.0, 200.0]),
            "lifted where pinned"
        );
        assert_eq!(m.pinned(), None);
    }

    #[test]
    fn the_players_touch_settings_drive_the_overlay() {
        use metallic_runtime::input::touch::StickMode;
        use metallic_runtime::input::{InputConfig, PlayerOverrides, TouchSettings};
        let mut rt = InputRuntime::new(&InputConfig::player_defaults());
        let mut o = PlayerOverrides::default();
        o.set_touch(TouchSettings {
            stick_mode: StickMode::Fixed,
            opacity: 0.5,
            size: 1.2,
            large_controls: true,
        });
        rt.set_overrides(o);
        let mut game = Game::from_world(
            "t".into(),
            metallic_core::World::new_with_services(),
            std::rc::Rc::new(metallic_runtime::StdoutSink),
        );
        let mut overlay = TouchOverlay::new(
            std::path::Path::new("."),
            Some(DeviceProfile::phone()),
            false,
        );
        overlay.update(&mut rt, &mut game, (1600, 739), 1.0, 1.0, 0.016);
        assert_eq!(overlay.controls.profile().stick_mode, StickMode::Fixed);
        assert!(overlay.controls.profile().large, "Large controls");
        assert_eq!(overlay.controls.profile().scale, 1.2);
        assert_eq!(overlay.opacity, 0.5);
    }

    /// #333: while touch shows, the game's GUI starts below the menu button.
    #[test]
    fn the_gui_starts_below_the_menu_button_while_touch_shows() {
        use metallic_runtime::input::InputConfig;
        let game = || {
            Game::from_world(
                "t".into(),
                metallic_core::World::new_with_services(),
                std::rc::Rc::new(metallic_runtime::StdoutSink),
            )
        };
        let mut rt = InputRuntime::new(&InputConfig::player_defaults());
        let mut g = game();
        let mut phone = TouchOverlay::new(
            std::path::Path::new("."),
            Some(DeviceProfile::phone()),
            false,
        );
        phone.update(&mut rt, &mut g, (1600, 739), 2.0, 1.0, 0.016);
        let menu = phone.controls.layout().menu.expect("the menu button");
        let bottom = menu.center[1] + menu.radius + 8.0;
        let want = (phone.view.origin[1] + bottom * phone.view.px_per_dp) / 2.0;
        assert!(
            (g.gui_inset() - want).abs() < 1e-3,
            "{} vs {want}",
            g.gui_inset()
        );
        assert!(g.gui_inset() > 0.0);
        // On a desktop with the keyboard in use, the touch controls hide and the GUI has the
        // whole screen.
        let mut rt = InputRuntime::new(&InputConfig::player_defaults());
        let mut g = game();
        let mut desktop = TouchOverlay::new(std::path::Path::new("."), None, false);
        desktop.update(&mut rt, &mut g, (1280, 720), 1.0, 1.0, 0.016);
        assert_eq!(g.gui_inset(), 0.0);
        // A touch arrives: the GUI eases down with the 140 ms fade instead of jumping.
        rt.set_forced_device(Some(DeviceKind::Touch));
        let mut seen = Vec::new();
        for i in 0..12 {
            desktop.update(
                &mut rt,
                &mut g,
                (1280, 720),
                1.0,
                0.1 + i as f64 * 0.016,
                0.016,
            );
            seen.push(g.gui_inset());
        }
        let full = gui_inset(&desktop.controls, &desktop.view, 1.0);
        assert!(full > 0.0);
        assert!(seen[0] > 0.0 && seen[0] < full * 0.25, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "{seen:?}");
        assert!((seen[11] - full).abs() < 1e-3, "{seen:?} vs {full}");
    }

    /// #339 review: a custom layout that puts the menu button at the bottom leaves the GUI alone.
    #[test]
    fn a_menu_button_at_the_bottom_leaves_the_gui_alone() {
        use metallic_runtime::input::{InputConfig, Role, TouchPlacement};
        let mut config = InputConfig::player_defaults();
        let menu = config
            .contexts
            .iter_mut()
            .flat_map(|c| c.actions.iter_mut())
            .find(|a| a.role == Role::Menu)
            .expect("the Menu action");
        // Bottom centre of the safe area.
        menu.touch_custom = Some(TouchPlacement {
            position: [0.5, -24.0, 1.0, -64.0],
            size: 48.0,
        });
        let mut rt = InputRuntime::new(&config);
        let mut g = Game::from_world(
            "t".into(),
            metallic_core::World::new_with_services(),
            std::rc::Rc::new(metallic_runtime::StdoutSink),
        );
        let mut phone = TouchOverlay::new(
            std::path::Path::new("."),
            Some(DeviceProfile::phone()),
            false,
        );
        phone.update(&mut rt, &mut g, (1600, 739), 2.0, 1.0, 0.016);
        let placed = phone.controls.layout().menu.expect("the menu button");
        assert!(placed.center[1] > 300.0, "at the bottom: {placed:?}");
        assert_eq!(g.gui_inset(), 0.0);
    }

    #[test]
    fn fade_takes_140_ms_and_is_instant_under_reduce_motion() {
        let mut a = 0.0;
        for _ in 0..7 {
            a = fade(a, 1.0, 0.01, FADE_SECONDS);
        }
        assert!(a > 0.45 && a < 0.55, "{a}");
        for _ in 0..10 {
            a = fade(a, 1.0, 0.01, FADE_SECONDS);
        }
        assert_eq!(a, 1.0);
        assert_eq!(fade(1.0, 0.0, 0.001, 0.0), 0.0, "reduce motion");
    }

    #[test]
    fn touch_images_must_stay_inside_the_project() {
        let p = Path::new("game");
        assert_eq!(
            project_file(p, "ui/jump.png"),
            Some(PathBuf::from("game/ui/jump.png"))
        );
        assert_eq!(project_file(p, "../secret.png"), None);
        assert_eq!(project_file(p, "rbxasset://x.png"), None);
        assert_eq!(project_file(p, ""), None);
        #[cfg(windows)]
        assert_eq!(project_file(p, "C:/x.png"), None);
    }
}
