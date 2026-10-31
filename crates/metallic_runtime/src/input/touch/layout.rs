//! The placement rule of spec 7.2: a pure function from the screen to control positions, in dp.
use super::stick::StickMode;
use super::{MAX_BUTTONS, TouchSet};

/// Smallest control: 48 dp (Android accessibility guidance, spec 7.2).
pub const MIN_CONTROL: f32 = 48.0;
/// Least space between the edges of two controls.
pub const MIN_GAP: f32 = 8.0;
/// Least space between a control and the edge of the safe area.
pub const SAFE_MARGIN: f32 = 16.0;
/// Distance of the primary button and the fixed stick from the safe-area corner.
pub const CORNER_INSET: f32 = 24.0;
/// The stick zone starts below this fraction of the safe area's height.
pub const ZONE_TOP: f32 = 0.15;
/// Ring radii around the primary button's centre (phone).
pub const RING_RADII: [f32; 2] = [84.0, 152.0];
/// Ring radii are multiplied by this on a tablet.
pub const TABLET_RING_FACTOR: f32 = 1.167;
/// Slot angles in degrees, 0 is right, counter-clockwise: ring 1 (S1 to S3), ring 2 (S4 to S6).
pub const RING_ANGLES: [[f32; 3]; 2] = [[180.0, 130.0, 80.0], [175.0, 140.0, 105.0]];
/// Where a button goes when its own slot is blocked (a narrow portrait screen): more angles on
/// ring 2, then on a third ring as far out again. Degrees.
const SPARE_RING2: [f32; 3] = [90.0, 157.5, 122.5];
const SPARE_RING3: [f32; 5] = [100.0, 120.0, 140.0, 160.0, 180.0];
/// The menu button: 48 dp, top centre of the safe area, 16 dp down.
pub const MENU_SIZE: f32 = 48.0;
/// Large controls: the smallest control on a phone (15 mm) and a tablet (24 mm), spec 7.2.
pub const LARGE_MIN: [f32; 2] = [94.0, 151.0];
/// The player's control size (spec 8.1, Touch tab): 0.8x to 1.5x, never below 48 dp.
pub const CONTROL_SCALE_RANGE: (f32, f32) = (0.8, 1.5);

/// A rectangle in dp, origin top-left, y down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from its corner and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The square around a circle.
    pub fn around(center: [f32; 2], radius: f32) -> Rect {
        Rect::new(
            center[0] - radius,
            center[1] - radius,
            2.0 * radius,
            2.0 * radius,
        )
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The centre.
    pub fn center(&self) -> [f32; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }

    /// The rectangle shrunk by `by` on every side (never below zero size).
    pub fn inset(&self, by: f32) -> Rect {
        Rect::new(
            self.x + by,
            self.y + by,
            (self.w - 2.0 * by).max(0.0),
            (self.h - 2.0 * by).max(0.0),
        )
    }

    /// True when `p` is inside (edges included).
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.right() && p[1] >= self.y && p[1] <= self.bottom()
    }

    /// True when `other` lies entirely inside, with `slack` dp of rounding allowed.
    pub fn contains_rect(&self, other: &Rect, slack: f32) -> bool {
        other.x >= self.x - slack
            && other.y >= self.y - slack
            && other.right() <= self.right() + slack
            && other.bottom() <= self.bottom() + slack
    }
}

/// The screen the controls are placed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceProfile {
    /// Screen size in dp.
    pub size: [f32; 2],
    /// Safe-area insets in dp: left, top, right, bottom (cutouts and system bars).
    pub insets: [f32; 4],
    /// Tablet sizes (spec 7.1 and 7.2: the larger column).
    pub tablet: bool,
    /// The Large controls preset (every control at least 15 mm on a phone, 24 mm on a tablet).
    pub large: bool,
    /// The player's control size: the stick and the buttons times this (0.8 to 1.5); the menu
    /// button keeps its size.
    pub scale: f32,
    /// Floating (default) or fixed stick.
    pub stick_mode: StickMode,
}

impl DeviceProfile {
    /// A phone-sized profile without insets.
    pub fn new(size: [f32; 2], insets: [f32; 4], tablet: bool) -> DeviceProfile {
        DeviceProfile {
            size,
            insets,
            tablet,
            large: false,
            scale: 1.0,
            stick_mode: StickMode::Floating,
        }
    }

    /// Small phone, landscape: 640 x 360 dp, no insets (spec 7.4).
    pub fn small_phone() -> DeviceProfile {
        DeviceProfile::new([640.0, 360.0], [0.0; 4], false)
    }

    /// Phone, landscape: 844 x 390 dp, notch insets 48 left and right, 20 at the bottom.
    pub fn phone() -> DeviceProfile {
        DeviceProfile::new([844.0, 390.0], [48.0, 0.0, 48.0, 20.0], false)
    }

    /// Tablet, landscape: 1180 x 820 dp, 20 at the bottom.
    pub fn tablet() -> DeviceProfile {
        DeviceProfile::new([1180.0, 820.0], [0.0, 0.0, 0.0, 20.0], true)
    }

    /// A device screen of `size` dp guessed from its shortest side: 600 dp and more is a tablet.
    pub fn for_screen(size: [f32; 2], insets: [f32; 4]) -> DeviceProfile {
        DeviceProfile::new(size, insets, size[0].min(size[1]) >= 600.0)
    }

    /// The same device held upright: width and height swap, the side insets move to the top
    /// (the notch) and the bottom inset stays (the home indicator).
    pub fn portrait(self) -> DeviceProfile {
        let [l, t, r, b] = self.insets;
        let (w, h) = (
            self.size[0].min(self.size[1]),
            self.size[0].max(self.size[1]),
        );
        DeviceProfile {
            size: [w, h],
            insets: [t, l.max(r), t, b],
            ..self
        }
    }

    /// The Large controls variant.
    pub fn with_large(self, large: bool) -> DeviceProfile {
        DeviceProfile { large, ..self }
    }

    /// With a control size, clamped to [`CONTROL_SCALE_RANGE`] (1 when not a number).
    pub fn with_scale(self, scale: f32) -> DeviceProfile {
        let scale = if scale.is_finite() {
            scale.clamp(CONTROL_SCALE_RANGE.0, CONTROL_SCALE_RANGE.1)
        } else {
            1.0
        };
        DeviceProfile { scale, ..self }
    }

    /// With a stick mode.
    pub fn with_stick_mode(self, stick_mode: StickMode) -> DeviceProfile {
        DeviceProfile { stick_mode, ..self }
    }

    /// The safe area: the screen minus the insets.
    pub fn safe_area(&self) -> Rect {
        let [l, t, r, b] = self
            .insets
            .map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
        let [w, h] = self
            .size
            .map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
        Rect::new(l, t, (w - l - r).max(0.0), (h - t - b).max(0.0))
    }
}

/// Control sizes for a profile, in dp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    /// Diameter of the primary button.
    pub primary: f32,
    /// Diameter of the other buttons.
    pub secondary: f32,
    /// Stick base radius R.
    pub stick_radius: f32,
    /// Knob diameter.
    pub knob: f32,
    /// Ring radii around the primary centre.
    pub rings: [f32; 3],
    /// Menu button diameter.
    pub menu: f32,
}

/// The sizes of spec 7.1 and 7.2 for `profile`; Large controls grow every control to the
/// minimum and the rings so that neighbours stay 8 dp apart.
pub fn sizes_for(profile: &DeviceProfile) -> Sizes {
    let t = usize::from(profile.tablet);
    // The player's size, never below 48 dp (a profile built by hand may hold any number).
    let k = if profile.scale.is_finite() {
        profile
            .scale
            .clamp(CONTROL_SCALE_RANGE.0, CONTROL_SCALE_RANGE.1)
    } else {
        1.0
    };
    let sized = |dp: f32| (dp * k).max(MIN_CONTROL);
    let mut primary: f32 = sized([72.0, 84.0][t]);
    let mut secondary: f32 = sized([56.0, 64.0][t]);
    let mut stick_radius: f32 = sized([60.0, 70.0][t] * 2.0) / 2.0;
    let knob = sized([56.0f32, 65.0][t]);
    let mut menu = MENU_SIZE;
    let factor = if profile.tablet {
        TABLET_RING_FACTOR
    } else {
        1.0
    };
    let mut r1 = RING_RADII[0] * factor * k;
    let mut r2 = RING_RADII[1] * factor * k;
    if profile.large {
        let min = LARGE_MIN[t];
        primary = primary.max(min);
        secondary = secondary.max(min);
        stick_radius = stick_radius.max(min / 2.0);
        menu = menu.max(min);
    }
    // Ring 1 clears the primary and its neighbours 50 degrees apart; ring 2 clears ring 1 and
    // its neighbours 35 degrees apart. (At the standard sizes the spec's radii already do.)
    let chord = |deg: f32| 2.0 * (deg.to_radians() / 2.0).sin();
    r1 = r1
        .max((primary + secondary) / 2.0 + MIN_GAP)
        .max((secondary + MIN_GAP) / chord(50.0));
    r2 = r2
        .max(r1 + secondary + MIN_GAP)
        .max((secondary + MIN_GAP) / chord(35.0));
    Sizes {
        primary,
        secondary,
        stick_radius,
        knob: knob.max(MIN_CONTROL),
        rings: [r1, r2, r2 + (r2 - r1)],
        menu,
    }
}

/// The stick as placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedStick {
    /// Where the base rests (the fixed position, and the hint ring of the floating stick).
    pub center: [f32; 2],
    /// Base radius R.
    pub radius: f32,
    /// Knob radius.
    pub knob_radius: f32,
}

/// A button as placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedButton {
    /// Index into [`TouchSet::buttons`] (or 0 for the menu).
    pub index: usize,
    /// Its control slot.
    pub slot: usize,
    /// Centre in dp.
    pub center: [f32; 2],
    /// Radius in dp (the hit area is this circle).
    pub radius: f32,
    /// The primary button (violet).
    pub primary: bool,
}

impl PlacedButton {
    /// The square around the button.
    pub fn rect(&self) -> Rect {
        Rect::around(self.center, self.radius)
    }
}

/// Where every touch control is, for one screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TouchLayout {
    /// The screen in dp.
    pub screen: Rect,
    /// The safe area.
    pub safe: Rect,
    /// Where a stick may start: the left half of the safe area below its top 15%.
    pub stick_zone: Rect,
    /// Where the look pad is: the right half of the safe area.
    pub look_zone: Rect,
    /// The stick, when an action uses it.
    pub stick: Option<PlacedStick>,
    /// The buttons.
    pub buttons: Vec<PlacedButton>,
    /// The menu button.
    pub menu: Option<PlacedButton>,
    /// Indices into [`TouchSet::buttons`] of buttons that did not fit.
    pub unplaced: Vec<usize>,
}

impl TouchLayout {
    /// Every drawn control as a rectangle: the stick base (at rest), the buttons, the menu.
    pub fn rects(&self) -> Vec<Rect> {
        self.stick
            .iter()
            .map(|s| Rect::around(s.center, s.radius))
            .chain(self.buttons.iter().map(PlacedButton::rect))
            .chain(self.menu.iter().map(PlacedButton::rect))
            .collect()
    }

    /// Every drawn control as a circle: (centre, radius).
    pub fn circles(&self) -> Vec<([f32; 2], f32)> {
        self.stick
            .iter()
            .map(|s| (s.center, s.radius))
            .chain(self.buttons.iter().map(|b| (b.center, b.radius)))
            .chain(self.menu.iter().map(|b| (b.center, b.radius)))
            .collect()
    }

    /// The button whose circle holds `p`.
    pub fn button_at(&self, p: [f32; 2]) -> Option<&PlacedButton> {
        self.buttons
            .iter()
            .chain(self.menu.iter())
            .find(|b| dist(b.center, p) <= b.radius)
    }

    /// True when `p` is within the 8 dp of inactive space around a button (spec 7.5).
    pub fn near_button(&self, p: [f32; 2]) -> bool {
        self.buttons
            .iter()
            .chain(self.menu.iter())
            .any(|b| dist(b.center, p) <= b.radius + MIN_GAP)
    }
}

pub(crate) fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn on_ring(center: [f32; 2], radius: f32, deg: f32) -> [f32; 2] {
    let a = deg.to_radians();
    // Screen y points down, so counter-clockwise means up.
    [center[0] + radius * a.cos(), center[1] - radius * a.sin()]
}

/// True when the circle fits inside `area` and keeps [`MIN_GAP`] to every circle in `taken`.
fn fits(area: &Rect, taken: &[([f32; 2], f32)], c: [f32; 2], r: f32) -> bool {
    const SLACK: f32 = 1e-3;
    area.contains_rect(&Rect::around(c, r), SLACK)
        && taken
            .iter()
            .all(|&(o, or)| dist(o, c) - or - r >= MIN_GAP - SLACK)
}

/// A custom placement: the top-left corner from `TouchPosition` (scale of the safe area plus
/// dp), the size from `TouchSize` (at least 48 dp), kept inside the safe area minus the margin.
fn custom_circle(
    area: &Rect,
    safe: &Rect,
    p: &super::TouchPlacement,
    default: f32,
) -> ([f32; 2], f32) {
    let size = if p.size > 0.0 { p.size } else { default };
    let size = size.max(MIN_CONTROL).min(area.w.min(area.h)).max(0.0);
    let r = size / 2.0;
    let x = safe.x + p.position[0] * safe.w + p.position[1] + r;
    let y = safe.y + p.position[2] * safe.h + p.position[3] + r;
    let clamp = |v: f32, lo: f32, hi: f32| {
        if lo > hi {
            (lo + hi) / 2.0
        } else {
            v.clamp(lo, hi)
        }
    };
    (
        [
            clamp(x, area.x + r, area.right() - r),
            clamp(y, area.y + r, area.bottom() - r),
        ],
        r,
    )
}

/// Places the controls of `set` on the screen of `profile` (spec 7.2).
///
/// - Stick: its rest position is the fixed stick, centre at safe-area left + 24 + R,
///   bottom - 24 - R.
/// - The first automatic button is the primary, centre at safe-area right - 24 - size/2,
///   bottom - 24 - size/2; the next go on ring 1 (84 dp, 180, 130, 80 degrees) and ring 2
///   (152 dp, 175, 140, 105 degrees); rings times 1.167 on a tablet.
/// - A button whose slot would break a rule (narrow portrait screens, Large controls) takes the
///   first spare slot that keeps every rule, else it is not placed ([`TouchLayout::unplaced`]).
/// - `TouchLayout = Custom` controls go where they say (inside the margin) and are placed first.
///
/// Rules every automatic control keeps: at least 48 dp, at least 8 dp from every other control,
/// at least 16 dp inside the safe area.
pub fn layout(profile: &DeviceProfile, set: &TouchSet) -> TouchLayout {
    let s = sizes_for(profile);
    let safe = profile.safe_area();
    let area = safe.inset(SAFE_MARGIN);
    let half = safe.w / 2.0;
    let zone_top = safe.h * ZONE_TOP;
    let mut out = TouchLayout {
        screen: Rect::new(0.0, 0.0, profile.size[0].max(0.0), profile.size[1].max(0.0)),
        safe,
        stick_zone: Rect::new(safe.x, safe.y + zone_top, half, safe.h - zone_top),
        look_zone: Rect::new(safe.x + half, safe.y, safe.w - half, safe.h),
        ..TouchLayout::default()
    };
    let mut taken: Vec<([f32; 2], f32)> = Vec::new();

    if let Some(stick) = &set.stick {
        let (center, radius) = match &stick.custom {
            Some(p) => custom_circle(&area, &safe, p, 2.0 * s.stick_radius),
            None => {
                let r = s.stick_radius.min(area.w.min(area.h) / 2.0).max(0.0);
                let c = [safe.x + CORNER_INSET + r, safe.bottom() - CORNER_INSET - r];
                (c, r)
            }
        };
        out.stick = Some(PlacedStick {
            center,
            radius,
            knob_radius: (s.knob / 2.0).min(radius),
        });
        taken.push((center, radius));
    }

    if let Some(menu) = &set.menu {
        let (center, r) = match &menu.custom {
            Some(p) => custom_circle(&area, &safe, p, s.menu),
            None => {
                let r = s.menu / 2.0;
                ([safe.x + safe.w / 2.0, safe.y + SAFE_MARGIN + r], r)
            }
        };
        if menu.custom.is_some() || fits(&area, &taken, center, r) {
            out.menu = Some(PlacedButton {
                index: 0,
                slot: menu.slot,
                center,
                radius: r,
                primary: false,
            });
            taken.push((center, r));
        }
    }

    // Custom buttons claim their space first.
    for (i, b) in set.buttons.iter().enumerate().take(MAX_BUTTONS) {
        if let Some(p) = &b.custom {
            let (center, radius) = custom_circle(&area, &safe, p, s.secondary);
            out.buttons.push(PlacedButton {
                index: i,
                slot: b.slot,
                center,
                radius,
                primary: false,
            });
            taken.push((center, radius));
        }
    }

    let pr = s.primary / 2.0;
    let hub = [
        safe.right() - CORNER_INSET - pr,
        safe.bottom() - CORNER_INSET - pr,
    ];
    // Spare slots (ring index, angle) after the six regular ones.
    let mut slots: Vec<(usize, f32)> = Vec::with_capacity(14);
    for (ring, angles) in RING_ANGLES.iter().enumerate() {
        slots.extend(angles.iter().map(|&a| (ring, a)));
    }
    slots.extend(SPARE_RING2.iter().map(|&a| (1, a)));
    slots.extend(SPARE_RING3.iter().map(|&a| (2, a)));
    let mut used = [false; 14];
    let mut auto = 0usize;
    for (i, b) in set.buttons.iter().enumerate() {
        if i >= MAX_BUTTONS {
            out.unplaced.push(i);
            continue;
        }
        if b.custom.is_some() {
            continue;
        }
        let placed = if auto == 0 {
            fits(&area, &taken, hub, pr).then_some((hub, pr, true))
        } else {
            let r = s.secondary / 2.0;
            let own = auto - 1;
            let order =
                std::iter::once(own).chain((MAX_BUTTONS..slots.len()).chain(0..MAX_BUTTONS));
            let mut found = None;
            for k in order {
                if used[k] {
                    continue;
                }
                let (ring, angle) = slots[k];
                let c = on_ring(hub, s.rings[ring], angle);
                if fits(&area, &taken, c, r) {
                    used[k] = true;
                    found = Some((c, r, false));
                    break;
                }
            }
            found
        };
        auto += 1;
        match placed {
            Some((center, radius, primary)) => {
                out.buttons.push(PlacedButton {
                    index: i,
                    slot: b.slot,
                    center,
                    radius,
                    primary,
                });
                taken.push((center, radius));
            }
            None => out.unplaced.push(i),
        }
    }
    out.buttons.sort_by_key(|b| b.index);
    out
}
