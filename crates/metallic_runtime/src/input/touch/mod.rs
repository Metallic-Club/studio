//! Automatic touch controls (docs/design/input-actions.md section 7).
//!
//! The touch screen is just another device: a thumbstick, a look pad and on-screen buttons
//! write virtual controls that bindings read, so a game never knows it is on a phone.
//!
//! - [`TouchSet`]: which actions get a touch control. Every enabled action with `ShowOnTouch`:
//!   a Direction2D action gets the stick or the look pad (`TouchSlot`), a Bool action a button,
//!   sorted by `TouchOrder` (ties: context priority, then tree order); at most
//!   [`MAX_BUTTONS`] buttons, plus the menu button for a Bool action with the `Menu` role.
//!   [`InputRuntime`](super::InputRuntime) builds it on every load and adds the implicit
//!   bindings: `TouchStick` (dead zone 0.15), `TouchLook` (0.01 rad per dp) and one
//!   `TouchButtonN` control per button.
//! - [`layout`]: the pure placement rule of spec 7.2, in dp, from a [`DeviceProfile`] (screen
//!   size and safe-area insets). Every control is at least 48 dp, 8 dp from every other control
//!   and 16 dp inside the safe area.
//! - [`Stick`]: floating or fixed thumbstick maths (radius R, travel R, the base follows past R).
//! - [`TouchControls`]: finger routing (spec 7.5). Each finger is captured by the first thing it
//!   lands on for its whole life; the controls turn fingers into [`RawEvent`](super::RawEvent)s
//!   for the runtime.
//! - [`paint`]: draws the overlay with egui in the visual-language colours.
//!
//! Units: dp (1/160 inch), origin at the top-left of the screen, y down. The Player converts
//! pixels with `px_per_dp`.

mod controls;
mod layout;
mod paint;
mod stick;
#[cfg(test)]
mod tests;

pub use controls::{Capture, MAX_FINGERS, TouchControls, TouchPhase};
pub use layout::{DeviceProfile, PlacedButton, PlacedStick, Rect, TouchLayout, layout, sizes_for};
/// What [`TouchControls::button_infos`] reports to the scripts.
pub use metallic_script::TouchButtonInfo;
pub use paint::{OverlayStyle, paint};
pub use stick::{Stick, StickMode};

use super::config::{ActionType, InputConfig, Role, TouchPlacement, TouchSlot};
use super::control::TOUCH_BUTTON_SLOTS;
use super::runtime::ActionId;
use metallic_core::InstanceId;

/// Most buttons the layout places around the primary (Studio warns above 5).
pub const MAX_BUTTONS: usize = 6;
/// Control slot of the menu button (`TouchButton7`).
pub const MENU_SLOT: usize = TOUCH_BUTTON_SLOTS - 1;
/// Dead zone of the touch stick, as a fraction of its radius (spec 7.1).
pub const TOUCH_STICK_DEADZONE: f32 = 0.15;
/// Radians the camera turns per dp dragged on the look pad (spec 6.1).
pub const TOUCH_LOOK_SCALE: f32 = 0.01;

/// What a button draws as its icon when it has no `TouchImage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchIcon {
    /// Two chevrons up.
    Jump,
    /// Two chevrons right.
    Run,
    /// A ring around a dot.
    Interact,
    /// A crosshair.
    Primary,
    /// A diamond.
    Secondary,
    /// Two bars.
    Menu,
    /// The first letter of the label.
    Letter,
}

impl TouchIcon {
    fn for_role(role: Role) -> Self {
        match role {
            Role::Jump => Self::Jump,
            Role::Run => Self::Run,
            Role::Interact => Self::Interact,
            Role::Primary => Self::Primary,
            Role::Secondary => Self::Secondary,
            Role::Menu => Self::Menu,
            _ => Self::Letter,
        }
    }
}

/// One on-screen button.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchButtonDef {
    /// The action it presses.
    pub action: ActionId,
    /// `Context/Action`.
    pub path: String,
    /// The `InputAction` instance (none for the built-in context).
    pub instance: Option<InstanceId>,
    /// Text under the button: `TouchLabel`, else `DisplayName`, else the action's name.
    pub label: String,
    /// `TouchImage` (empty: draw [`icon`](Self::icon)).
    pub image: String,
    /// The icon for the action's role.
    pub icon: TouchIcon,
    /// `TouchLayout = Custom`.
    pub custom: Option<TouchPlacement>,
    /// Its control: `TouchButton1` + slot ([`MENU_SLOT`] for the menu button).
    pub slot: usize,
}

/// The stick or the look pad.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchAxisDef {
    /// The action it drives.
    pub action: ActionId,
    /// `Context/Action`.
    pub path: String,
    /// `TouchLayout = Custom` (the stick's fixed position and size).
    pub custom: Option<TouchPlacement>,
}

/// Which actions get which touch control. Built by the runtime from the contexts it loaded.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TouchSet {
    /// The left thumbstick (`TouchStick`).
    pub stick: Option<TouchAxisDef>,
    /// The look pad (`TouchLook`).
    pub look: Option<TouchAxisDef>,
    /// Buttons in placement order: the first is the primary. Slot `i` is `TouchButton{i+1}`.
    pub buttons: Vec<TouchButtonDef>,
    /// The menu button at the top centre (a Bool action with role `Menu`).
    pub menu: Option<TouchButtonDef>,
    /// Paths of `ShowOnTouch` actions that got no control (more than [`MAX_BUTTONS`] buttons,
    /// a second stick, or a type with no touch control).
    pub left_out: Vec<String>,
}

/// What the touch controls give one action of the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TouchUse {
    Stick,
    Look,
    Button(usize),
}

impl TouchSet {
    /// Builds the set from `config`. `rank` orders the contexts (0 is asked first; `u16::MAX`
    /// for a disabled context), the same order the runtime uses for arbitration. Returns the
    /// set (action ids still 0) and, per context and action, what it got.
    pub(crate) fn build(
        config: &InputConfig,
        rank: &[u16],
    ) -> (TouchSet, Vec<Vec<Option<TouchUse>>>) {
        let mut uses: Vec<Vec<Option<TouchUse>>> = config
            .contexts
            .iter()
            .map(|c| vec![None; c.actions.len()])
            .collect();
        let mut set = TouchSet::default();
        // Candidates in arbitration order: higher priority contexts claim the stick first.
        let mut order: Vec<(u16, usize, usize)> = Vec::new();
        for (ci, ctx) in config.contexts.iter().enumerate() {
            if !ctx.enabled || rank.get(ci).copied().unwrap_or(u16::MAX) == u16::MAX {
                continue;
            }
            for (ai, a) in ctx.actions.iter().enumerate() {
                if a.enabled && a.show_on_touch {
                    order.push((rank[ci], ci, ai));
                }
            }
        }
        order.sort_by_key(|&(r, ci, ai)| (r, ci, ai));
        let path = |ci: usize, ai: usize| {
            format!(
                "{}/{}",
                config.contexts[ci].name, config.contexts[ci].actions[ai].name
            )
        };
        let mut buttons: Vec<(f32, u16, usize, usize)> = Vec::new();
        for &(r, ci, ai) in &order {
            let a = &config.contexts[ci].actions[ai];
            let axis = |ci, ai| TouchAxisDef {
                action: ActionId(0),
                path: path(ci, ai),
                custom: a.touch_custom,
            };
            match a.ty {
                ActionType::Direction2D => {
                    let want = match (a.touch_slot, a.role) {
                        (TouchSlot::Stick, _) | (TouchSlot::Auto, Role::Move) => [true, false],
                        (TouchSlot::Look, _) | (TouchSlot::Auto, Role::Look) => [false, true],
                        (TouchSlot::Auto, _) => [true, true],
                    };
                    if want[0] && set.stick.is_none() {
                        set.stick = Some(axis(ci, ai));
                        uses[ci][ai] = Some(TouchUse::Stick);
                    } else if want[1] && set.look.is_none() {
                        set.look = Some(axis(ci, ai));
                        uses[ci][ai] = Some(TouchUse::Look);
                    } else {
                        set.left_out.push(path(ci, ai));
                    }
                }
                ActionType::Bool if a.role == Role::Menu && set.menu.is_none() => {
                    set.menu = Some(button_def(config, ci, ai, MENU_SLOT));
                    uses[ci][ai] = Some(TouchUse::Button(MENU_SLOT));
                }
                ActionType::Bool => buttons.push((a.touch_order, r, ci, ai)),
                _ => set.left_out.push(path(ci, ai)),
            }
        }
        buttons.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then((a.1, a.2, a.3).cmp(&(b.1, b.2, b.3)))
        });
        for (i, &(_, _, ci, ai)) in buttons.iter().enumerate() {
            if i < MAX_BUTTONS {
                set.buttons.push(button_def(config, ci, ai, i));
                uses[ci][ai] = Some(TouchUse::Button(i));
            } else {
                set.left_out.push(path(ci, ai));
            }
        }
        (set, uses)
    }

    /// Fills in the runtime's action ids by path.
    pub(crate) fn resolve(&mut self, find: impl Fn(&str) -> Option<ActionId>) {
        for axis in [self.stick.as_mut(), self.look.as_mut()]
            .into_iter()
            .flatten()
        {
            axis.action = find(&axis.path).unwrap_or(ActionId(u16::MAX));
        }
        for b in self.buttons.iter_mut().chain(self.menu.as_mut()) {
            b.action = find(&b.path).unwrap_or(ActionId(u16::MAX));
        }
    }

    /// The button on control slot `slot`.
    pub fn button_in_slot(&self, slot: usize) -> Option<&TouchButtonDef> {
        if slot == MENU_SLOT {
            self.menu.as_ref()
        } else {
            self.buttons.get(slot)
        }
    }

    /// True when nothing would be drawn.
    pub fn is_empty(&self) -> bool {
        self.stick.is_none() && self.buttons.is_empty() && self.menu.is_none()
    }
}

fn button_def(config: &InputConfig, ci: usize, ai: usize, slot: usize) -> TouchButtonDef {
    let ctx = &config.contexts[ci];
    let a = &ctx.actions[ai];
    let label = [&a.touch_label, &a.display_name, &a.name]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
    TouchButtonDef {
        action: ActionId(0),
        path: format!("{}/{}", ctx.name, a.name),
        instance: a.instance,
        label,
        image: a.touch_image.clone(),
        icon: TouchIcon::for_role(a.role),
        custom: a.touch_custom,
        slot,
    }
}
