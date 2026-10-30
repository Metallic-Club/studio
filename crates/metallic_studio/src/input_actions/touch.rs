//! The touch layout model: device presets, the automatic layout (spec 7.1, 7.2), custom
//! positions (`TouchLayout`, `TouchPosition`, `TouchSize`) and the editing rules (spec 7.3).
//!
//! Everything is in dp. Positions are stored relative to the safe area so one layout works on a
//! phone and a tablet: each axis is anchored to the nearer edge of the safe area (Scale 0 or 1)
//! with an Offset in dp from that edge, and the point stored is the control's centre.
//!
//! The Player's overlay (#252) is meant to draw from the same layout; until it lands, Studio's
//! device preview is the only user of this module.

use metallic_core::{InstanceId, UDim2, Value, World};
use metallic_runtime::input::ActionType;

use super::commands::StepBuilder;
use super::model::{InputModel, TouchSlot, touch_props};

/// Smallest control the editor allows (spec 7.2).
pub const MIN_SIZE: f32 = 48.0;
/// Smallest gap between two control edges.
pub const MIN_GAP: f32 = 8.0;
/// Smallest distance from a control to the safe-area edge.
pub const SAFE_MARGIN: f32 = 16.0;
/// Drag snap grid.
pub const GRID: f32 = 4.0;
/// Shift+arrow nudge.
pub const BIG_NUDGE: f32 = 16.0;
/// Most buttons placed; Studio warns above [`WARN_BUTTONS`].
pub const MAX_BUTTONS: usize = 6;
/// Buttons above this count get a warning.
pub const WARN_BUTTONS: usize = 5;
/// Largest size a control can be given in the editor.
pub const MAX_SIZE: f32 = 200.0;

/// Safe-area insets in dp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Insets {
    /// Left.
    pub left: f32,
    /// Top.
    pub top: f32,
    /// Right.
    pub right: f32,
    /// Bottom.
    pub bottom: f32,
}

/// A rectangle in dp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DpRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl DpRect {
    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The rect shrunk by `m` on every side.
    pub fn shrink(&self, m: f32) -> DpRect {
        DpRect {
            x: self.x + m,
            y: self.y + m,
            w: (self.w - 2.0 * m).max(0.0),
            h: (self.h - 2.0 * m).max(0.0),
        }
    }
}

/// Screen orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    /// Wider than tall.
    Landscape,
    /// Taller than wide.
    Portrait,
}

/// Device presets for the preview (spec 7.4: typical sizes, not measurements of one product).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevicePreset {
    /// 640 x 360 dp, no insets.
    SmallPhone,
    /// 844 x 390 dp, insets 48, 0, 48, 20.
    Phone,
    /// 1180 x 820 dp, insets 0, 0, 0, 20.
    Tablet,
}

impl DevicePreset {
    /// All presets.
    pub const ALL: [DevicePreset; 3] = [
        DevicePreset::SmallPhone,
        DevicePreset::Phone,
        DevicePreset::Tablet,
    ];

    /// Name shown in the preset picker.
    pub fn label(self) -> &'static str {
        match self {
            DevicePreset::SmallPhone => "Small phone",
            DevicePreset::Phone => "Phone",
            DevicePreset::Tablet => "Tablet",
        }
    }

    /// The screen in this orientation.
    pub fn profile(self, orientation: Orientation) -> DeviceProfile {
        let (w, h, insets, tablet) = match self {
            DevicePreset::SmallPhone => (640.0, 360.0, [0.0, 0.0, 0.0, 0.0], false),
            DevicePreset::Phone => (844.0, 390.0, [48.0, 0.0, 48.0, 20.0], false),
            DevicePreset::Tablet => (1180.0, 820.0, [0.0, 0.0, 0.0, 20.0], true),
        };
        let [l, t, r, b] = insets;
        match orientation {
            Orientation::Landscape => DeviceProfile {
                preset: self,
                orientation,
                width: w,
                height: h,
                insets: Insets {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                },
                tablet,
            },
            // Turned upright: the side cutout moves to the top, the home indicator stays at the bottom.
            Orientation::Portrait => DeviceProfile {
                preset: self,
                orientation,
                width: h,
                height: w,
                insets: Insets {
                    left: 0.0,
                    top: l.max(r),
                    right: 0.0,
                    bottom: b,
                },
                tablet,
            },
        }
    }
}

/// One screen to lay out on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceProfile {
    /// Which preset.
    pub preset: DevicePreset,
    /// Orientation.
    pub orientation: Orientation,
    /// Width in dp.
    pub width: f32,
    /// Height in dp.
    pub height: f32,
    /// Safe-area insets.
    pub insets: Insets,
    /// Tablet sizes (spec 7.1, 7.2).
    pub tablet: bool,
}

impl DeviceProfile {
    /// "Phone landscape".
    pub fn label(&self) -> String {
        let o = match self.orientation {
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
        };
        format!("{} {o}", self.preset.label())
    }

    /// The screen minus the insets.
    pub fn safe_area(&self) -> DpRect {
        DpRect {
            x: self.insets.left,
            y: self.insets.top,
            w: self.width - self.insets.left - self.insets.right,
            h: self.height - self.insets.top - self.insets.bottom,
        }
    }

    /// Stick base radius.
    pub fn stick_radius(&self) -> f32 {
        if self.tablet { 70.0 } else { 60.0 }
    }

    /// Primary button diameter.
    pub fn primary_size(&self) -> f32 {
        if self.tablet { 84.0 } else { 72.0 }
    }

    /// Secondary button diameter.
    pub fn secondary_size(&self) -> f32 {
        if self.tablet { 64.0 } else { 56.0 }
    }

    /// The zone where a floating stick starts: left half of the safe area below its top 15%.
    pub fn stick_zone(&self) -> DpRect {
        let s = self.safe_area();
        let top = s.y + s.h * 0.15;
        DpRect {
            x: s.x,
            y: top,
            w: s.w / 2.0,
            h: s.bottom() - top,
        }
    }

    /// Every preset and orientation the checks look at.
    pub fn all() -> Vec<DeviceProfile> {
        DevicePreset::ALL
            .iter()
            .flat_map(|p| {
                [Orientation::Landscape, Orientation::Portrait]
                    .into_iter()
                    .map(move |o| p.profile(o))
            })
            .collect()
    }
}

/// What a placed control is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRole {
    /// The movement stick (fixed position shown; floating sticks start anywhere in the zone).
    Stick,
    /// The lowest `TouchOrder` button.
    Primary,
    /// Any other button.
    Secondary,
    /// The Player's menu button (built in, not editable).
    Menu,
}

/// A touch action as the layout needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchAction {
    /// The `InputAction` (None for built-in actions).
    pub id: Option<InstanceId>,
    /// `Context/Action`.
    pub path: String,
    /// Text on the button: `TouchLabel`, else `DisplayName`, else the name.
    pub label: String,
    /// Stick or button.
    pub role: ControlRole,
    /// `TouchLayout = Custom` with this centre and size.
    pub custom: Option<(UDim2, f32)>,
}

/// A placed control.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The action (None for built-in actions and the menu button).
    pub id: Option<InstanceId>,
    /// `Context/Action` (empty for the menu button).
    pub path: String,
    /// Text on it.
    pub label: String,
    /// What it is.
    pub role: ControlRole,
    /// Centre in dp from the screen's top left.
    pub center: [f32; 2],
    /// Diameter in dp.
    pub size: f32,
    /// Placed by `TouchPosition` rather than the automatic rule.
    pub custom: bool,
}

impl Placed {
    /// Radius.
    pub fn radius(&self) -> f32 {
        self.size / 2.0
    }

    /// Distance between the edges of two circular controls (negative when they overlap).
    pub fn gap(&self, other: &Placed) -> f32 {
        let dx = self.center[0] - other.center[0];
        let dy = self.center[1] - other.center[1];
        (dx * dx + dy * dy).sqrt() - self.radius() - other.radius()
    }

    /// True when the control is the menu button (fixed by the Player).
    pub fn editable(&self) -> bool {
        self.role != ControlRole::Menu
    }
}

/// The touch actions of a world, in the order the overlay places them: the stick first, then
/// buttons by `TouchOrder` (ties keep tree order). Buttons past [`MAX_BUTTONS`] are left out.
pub fn touch_actions(world: &World) -> Vec<TouchAction> {
    let model = InputModel::read(world);
    let mut stick = None;
    let mut sticks_seen = 0;
    let mut buttons: Vec<(f32, usize, TouchAction)> = Vec::new();
    for (i, (ctx, a)) in model.actions().enumerate() {
        if !ctx.enabled || !a.enabled || !a.show_on_touch {
            continue;
        }
        let props = touch_props(world, a.instance);
        let label = match a.instance {
            Some(id) => {
                let t = super::model::prop_string(world, id, "TouchLabel");
                if !t.is_empty() {
                    t
                } else if !a.display_name.is_empty() {
                    a.display_name.clone()
                } else {
                    a.name.clone()
                }
            }
            None => a.name.clone(),
        };
        let custom = props.custom.then_some((props.position, props.size));
        let path = format!("{}/{}", ctx.name, a.name);
        match a.ty {
            ActionType::Direction2D => {
                let is_stick = match props.slot {
                    TouchSlot::Stick => true,
                    TouchSlot::Look => false,
                    TouchSlot::Auto => sticks_seen == 0,
                };
                sticks_seen += 1;
                if is_stick && stick.is_none() {
                    stick = Some(TouchAction {
                        id: a.instance,
                        path,
                        label,
                        role: ControlRole::Stick,
                        custom,
                    });
                }
            }
            ActionType::Bool => buttons.push((
                a.touch_order,
                i,
                TouchAction {
                    id: a.instance,
                    path,
                    label,
                    role: ControlRole::Secondary,
                    custom,
                },
            )),
            _ => {}
        }
    }
    buttons.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<TouchAction> = stick.into_iter().collect();
    for (n, (_, _, mut b)) in buttons.into_iter().take(MAX_BUTTONS).enumerate() {
        if n == 0 {
            b.role = ControlRole::Primary;
        }
        out.push(b);
    }
    out
}

/// Number of enabled Bool actions with `ShowOnTouch` (before the [`MAX_BUTTONS`] cut).
pub fn touch_button_count(world: &World) -> usize {
    let model = InputModel::read(world);
    model
        .actions()
        .filter(|(c, a)| c.enabled && a.enabled && a.show_on_touch && a.ty == ActionType::Bool)
        .count()
}

/// Ring slots around the primary button: (radius, angle in degrees, 0 is right, counter-clockwise).
const RING: [(f32, f32); 6] = [
    (84.0, 180.0),
    (84.0, 130.0),
    (84.0, 80.0),
    (152.0, 175.0),
    (152.0, 140.0),
    (152.0, 105.0),
];

/// Lays out the controls on one screen (spec 7.1, 7.2). The menu button is always included.
pub fn layout(profile: &DeviceProfile, actions: &[TouchAction]) -> Vec<Placed> {
    let safe = profile.safe_area();
    let ring_factor = if profile.tablet { 1.167 } else { 1.0 };
    let mut out = Vec::new();
    let primary_size = profile.primary_size();
    let primary_center = [
        safe.right() - 24.0 - primary_size / 2.0,
        safe.bottom() - 24.0 - primary_size / 2.0,
    ];
    let mut ring = 0;
    for a in actions {
        let (auto_center, auto_size) = match a.role {
            ControlRole::Stick => {
                let r = profile.stick_radius();
                ([safe.x + 24.0 + r, safe.bottom() - 24.0 - r], 2.0 * r)
            }
            ControlRole::Primary => (primary_center, primary_size),
            ControlRole::Secondary => {
                let (radius, angle) = RING[ring.min(RING.len() - 1)];
                ring += 1;
                let (s, c) = angle.to_radians().sin_cos();
                let r = radius * ring_factor;
                (
                    [primary_center[0] + r * c, primary_center[1] - r * s],
                    profile.secondary_size(),
                )
            }
            ControlRole::Menu => continue,
        };
        let (center, size, custom) = match a.custom {
            Some((pos, size)) => (
                from_udim(pos, &safe),
                if size > 0.0 { size } else { auto_size },
                true,
            ),
            None => (auto_center, auto_size, false),
        };
        out.push(Placed {
            id: a.id,
            path: a.path.clone(),
            label: a.label.clone(),
            role: a.role,
            center,
            size,
            custom,
        });
    }
    out.push(Placed {
        id: None,
        path: String::new(),
        label: "Menu".into(),
        role: ControlRole::Menu,
        center: [safe.x + safe.w / 2.0, safe.y + 16.0 + MIN_SIZE / 2.0],
        size: MIN_SIZE,
        custom: false,
    });
    out
}

/// `TouchPosition` for a centre: each axis anchored to the nearer safe-area edge.
pub fn to_udim(center: [f32; 2], safe: &DpRect) -> UDim2 {
    let axis = |v: f32, start: f32, len: f32| -> (f64, f64) {
        if v - start < len / 2.0 {
            (0.0, f64::from(v - start))
        } else {
            (1.0, f64::from(v - start - len))
        }
    };
    let (xs, xo) = axis(center[0], safe.x, safe.w);
    let (ys, yo) = axis(center[1], safe.y, safe.h);
    UDim2::new(xs, xo.round(), ys, yo.round())
}

/// The centre a `TouchPosition` gives on a safe area.
pub fn from_udim(pos: UDim2, safe: &DpRect) -> [f32; 2] {
    [
        safe.x + pos.x_scale as f32 * safe.w + pos.x_offset as f32,
        safe.y + pos.y_scale as f32 * safe.h + pos.y_offset as f32,
    ]
}

/// Rounds to the 4 dp grid.
pub fn snap(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

/// Keeps a control of `size` inside the safe area minus the margin.
pub fn clamp_center(center: [f32; 2], size: f32, safe: &DpRect) -> [f32; 2] {
    let inner = safe.shrink(SAFE_MARGIN);
    let r = size / 2.0;
    let clamp = |v: f32, lo: f32, hi: f32| {
        if lo > hi {
            (lo + hi) / 2.0
        } else {
            v.clamp(lo, hi)
        }
    };
    [
        clamp(center[0], inner.x + r, inner.right() - r),
        clamp(center[1], inner.y + r, inner.bottom() - r),
    ]
}

/// A size the editor accepts: at least [`MIN_SIZE`], at most [`MAX_SIZE`], on the grid.
pub fn clamp_size(size: f32) -> f32 {
    snap(size).clamp(MIN_SIZE, MAX_SIZE)
}

/// What is wrong with a layout on one screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Violation {
    /// Smaller than [`MIN_SIZE`].
    TooSmall {
        /// Index into the layout.
        control: usize,
    },
    /// Closer than [`MIN_GAP`] to another control.
    TooClose {
        /// First index.
        a: usize,
        /// Second index.
        b: usize,
        /// The gap in dp (negative: overlap).
        gap: f32,
    },
    /// Closer than [`SAFE_MARGIN`] to the safe-area edge, or outside it.
    OutsideSafeArea {
        /// Index into the layout.
        control: usize,
    },
}

/// Checks a layout against the rules of spec 7.2.
pub fn violations(profile: &DeviceProfile, placed: &[Placed]) -> Vec<Violation> {
    let inner = profile.safe_area().shrink(SAFE_MARGIN);
    let mut out = Vec::new();
    for (i, p) in placed.iter().enumerate() {
        if p.size < MIN_SIZE - 0.01 {
            out.push(Violation::TooSmall { control: i });
        }
        let r = p.radius();
        let eps = 0.01;
        if p.center[0] - r < inner.x - eps
            || p.center[0] + r > inner.right() + eps
            || p.center[1] - r < inner.y - eps
            || p.center[1] + r > inner.bottom() + eps
        {
            out.push(Violation::OutsideSafeArea { control: i });
        }
    }
    for i in 0..placed.len() {
        for j in i + 1..placed.len() {
            let gap = placed[i].gap(&placed[j]);
            if gap < MIN_GAP - 0.01 {
                out.push(Violation::TooClose { a: i, b: j, gap });
            }
        }
    }
    out
}

/// Writes one control's placement as one undo step's worth of edits: `TouchLayout = Custom`,
/// `TouchPosition` and `TouchSize`.
pub fn write_custom(
    b: &mut StepBuilder<'_>,
    id: InstanceId,
    center: [f32; 2],
    size: f32,
    profile: &DeviceProfile,
) -> Result<(), metallic_core::CoreError> {
    let safe = profile.safe_area();
    b.set(id, "TouchLayout", Value::String("Custom".into()))?;
    b.set(id, "TouchPosition", Value::UDim2(to_udim(center, &safe)))?;
    b.set(id, "TouchSize", Value::Number(f64::from(clamp_size(size))))?;
    Ok(())
}

/// "Reset layout": every touch action back to `TouchLayout = Auto`.
pub fn reset_all(b: &mut StepBuilder<'_>) -> Result<usize, metallic_core::CoreError> {
    let ids: Vec<InstanceId> = touch_actions(b.world())
        .into_iter()
        .filter(|a| a.custom.is_some())
        .filter_map(|a| a.id)
        .collect();
    for &id in &ids {
        b.set(id, "TouchLayout", Value::String("Auto".into()))?;
        b.set(
            id,
            "TouchPosition",
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0)),
        )?;
        b.set(id, "TouchSize", Value::Number(0.0))?;
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto_actions(buttons: usize) -> Vec<TouchAction> {
        let mut v = vec![TouchAction {
            id: None,
            path: "C/Move".into(),
            label: "Move".into(),
            role: ControlRole::Stick,
            custom: None,
        }];
        for i in 0..buttons {
            v.push(TouchAction {
                id: None,
                path: format!("C/B{i}"),
                label: format!("B{i}"),
                role: if i == 0 {
                    ControlRole::Primary
                } else {
                    ControlRole::Secondary
                },
                custom: None,
            });
        }
        v
    }

    #[test]
    fn automatic_layout_keeps_the_rules_in_landscape() {
        // The spec verified the rule by hand for these screens (section 7.2).
        for preset in DevicePreset::ALL {
            let p = preset.profile(Orientation::Landscape);
            for n in 1..=MAX_BUTTONS {
                let placed = layout(&p, &auto_actions(n));
                assert!(
                    violations(&p, &placed).is_empty(),
                    "{} with {n} buttons: {:?}",
                    p.label(),
                    violations(&p, &placed)
                );
            }
        }
    }

    #[test]
    fn parkour_buttons_fit_every_screen() {
        // Move, Jump, Run: the built-in controls must pass on every preset and orientation.
        for p in DeviceProfile::all() {
            let placed = layout(&p, &auto_actions(2));
            assert!(violations(&p, &placed).is_empty(), "{}", p.label());
        }
    }

    #[test]
    fn udim_round_trip_on_every_screen() {
        let p = DevicePreset::Phone.profile(Orientation::Landscape);
        let safe = p.safe_area();
        for c in [[100.0, 100.0], [700.0, 300.0], [400.0, 60.0]] {
            let u = to_udim(c, &safe);
            assert!(u.x_scale == 0.0 || u.x_scale == 1.0);
            let back = from_udim(u, &safe);
            assert!((back[0] - c[0]).abs() < 0.51 && (back[1] - c[1]).abs() < 0.51);
        }
        // A button 60 dp from the bottom-right corner stays 60 dp from it on a tablet.
        let u = to_udim([safe.right() - 60.0, safe.bottom() - 60.0], &safe);
        let t = DevicePreset::Tablet.profile(Orientation::Landscape);
        let ts = t.safe_area();
        let c = from_udim(u, &ts);
        assert_eq!(c, [ts.right() - 60.0, ts.bottom() - 60.0]);
    }

    #[test]
    fn snapping_and_sizes() {
        assert_eq!(snap(13.0), 12.0);
        assert_eq!(snap(14.1), 16.0);
        assert_eq!(clamp_size(20.0), MIN_SIZE);
        assert_eq!(clamp_size(1000.0), MAX_SIZE);
        let safe = DevicePreset::Phone
            .profile(Orientation::Landscape)
            .safe_area();
        let c = clamp_center([0.0, 0.0], 60.0, &safe);
        assert_eq!(
            c,
            [safe.x + SAFE_MARGIN + 30.0, safe.y + SAFE_MARGIN + 30.0]
        );
    }

    #[test]
    fn violations_report_overlap_size_and_edges() {
        let p = DevicePreset::Phone.profile(Orientation::Landscape);
        let mut placed = layout(&p, &auto_actions(2));
        placed[1].center = placed[2].center; // primary on top of the secondary
        placed[0].size = 40.0;
        placed[0].center = [p.insets.left + 10.0, 200.0];
        let v = violations(&p, &placed);
        assert!(v.contains(&Violation::TooSmall { control: 0 }));
        assert!(v.contains(&Violation::OutsideSafeArea { control: 0 }));
        assert!(
            v.iter()
                .any(|x| matches!(x, Violation::TooClose { a: 1, b: 2, .. }))
        );
    }
}
