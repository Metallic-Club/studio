//! Colours and sizes of the Player menu, from `design/tokens/metallic.tokens.json` (graphite and
//! silver, violet only for focus, the selected tab and the one primary choice).
use egui::Color32;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `role.bg-deep` (g900): wells, keycap faces.
pub const BG_DEEP: Color32 = hex(0x111015);
/// `role.bg-base` (g850).
pub const BG_BASE: Color32 = hex(0x16151b);
/// `role.bg-panel` (g800): the menu window.
pub const BG_PANEL: Color32 = hex(0x1c1b22);
/// `role.bg-raised` (g750): rows, keycaps.
pub const BG_RAISED: Color32 = hex(0x24222b);
/// `role.bg-hover` (g700).
pub const BG_HOVER: Color32 = hex(0x2c2a34);
/// `role.bg-active` (g650).
pub const BG_ACTIVE: Color32 = hex(0x35323e);
/// `role.select` (v800): the selected tab and segment.
pub const SELECT: Color32 = hex(0x3a3354);
/// `role.line`.
pub const LINE: Color32 = hex(0x2b2933);
/// `role.line-strong` (g600).
pub const LINE_STRONG: Color32 = hex(0x43404d);
/// `role.line-control`: keycap and control outlines.
pub const LINE_CONTROL: Color32 = hex(0x6e6a79);
/// `role.text-1`.
pub const TEXT_1: Color32 = hex(0xe6e4ea);
/// `role.text-2`.
pub const TEXT_2: Color32 = hex(0xaeaab6);
/// `role.text-3`.
pub const TEXT_3: Color32 = hex(0x8e8a97);
/// `role.text-off` (g500).
pub const TEXT_OFF: Color32 = hex(0x5f5b6b);
/// `role.icon`: glyph strokes.
pub const ICON: Color32 = hex(0xbdbac4);
/// `role.accent` (v400): the primary choice.
pub const ACCENT: Color32 = hex(0x9d8be0);
/// `role.focus` (v300): the focus ring.
pub const FOCUS: Color32 = hex(0xb4a6ec);
/// `role.on-accent` (g900): text on violet.
pub const ON_ACCENT: Color32 = hex(0x111015);
/// `role.warning`: unbound slots.
pub const WARNING: Color32 = hex(0xe0ad55);
/// `role.warning-bg`.
pub const WARNING_BG: Color32 = hex(0x352c1d);

/// The scrim over the game while the menu is open.
pub const SCRIM: Color32 = Color32::from_rgba_premultiplied(5, 5, 7, 184);

/// Radii: keycaps and chips, widgets, menus, windows.
pub const R_XS: f32 = 3.0;
/// Widget radius.
pub const R_SM: f32 = 4.0;
/// Menu and toast radius.
pub const R_MD: f32 = 6.0;
/// Window radius.
pub const R_LG: f32 = 10.0;
/// The cut on the top-right corner of the menu window.
pub const CUT: f32 = 14.0;
/// Focus ring width.
pub const FOCUS_W: f32 = 2.0;
