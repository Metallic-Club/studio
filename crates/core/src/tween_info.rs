//! `TweenInfo` (#810): how a tween plays, as Roblox's data type of that name. Scripts make one with
//! `TweenInfo.new(...)` and pass it to `TweenService:Create`.

use crate::EnumItem;

/// How a tween plays: for how long, along which curve, how often and after what delay.
#[derive(Clone, Debug, PartialEq)]
pub struct TweenInfo {
    /// Seconds one way takes.
    pub time: f64,
    /// An `Enum.EasingStyle` item: the curve.
    pub easing_style: EnumItem,
    /// An `Enum.EasingDirection` item: which end of the curve is eased.
    pub easing_direction: EnumItem,
    /// How many times it plays again after the first; below 0 for ever.
    pub repeat_count: i32,
    /// Whether each play goes back to the start after reaching the goal.
    pub reverses: bool,
    /// Seconds to wait before each play.
    pub delay_time: f64,
}

impl Default for TweenInfo {
    /// Roblox's `TweenInfo.new()`: one second, `Quad`, `Out`, once, no reverse, no delay.
    fn default() -> Self {
        Self {
            time: 1.0,
            easing_style: EnumItem::new("EasingStyle", "Quad"),
            easing_direction: EnumItem::new("EasingDirection", "Out"),
            repeat_count: 0,
            reverses: false,
            delay_time: 0.0,
        }
    }
}
